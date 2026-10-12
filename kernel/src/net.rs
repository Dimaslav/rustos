//! Минимальный сетевой стек: Ethernet → ARP → IPv4 → ICMP.
//!
//! Пока только то, что нужно для ping: ARP-запрос/ответ, ICMP echo
//! request/reply. Ни TCP, ни UDP, ни DNS.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, Ordering};
use spin::Mutex;

pub const ETH_TYPE_ARP:  u16 = 0x0806;
pub const ETH_TYPE_IPV4: u16 = 0x0800;

pub const BROADCAST_MAC: [u8; 6] = [0xFF; 6];

/// Наш IP. В QEMU SLIRP гостю всегда выдаётся 10.0.2.15.
pub const OUR_IP: [u8; 4] = [10, 0, 2, 15];
/// Шлюз по умолчанию (он же хост).
pub const GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];
/// DNS-сервер SLIRP.
pub const DNS_IP: [u8; 4] = [10, 0, 2, 3];

const ARP_TABLE_MAX: usize = 8;

#[derive(Clone, Copy)]
struct ArpEntry {
    ip: [u8; 4],
    mac: [u8; 6],
}

static ARP_TABLE: Mutex<Vec<ArpEntry>> = Mutex::new(Vec::new());
static IP_ID: AtomicU16 = AtomicU16::new(1);

/// Найти MAC по IP. Только таблица, без сетевого запроса.
pub fn arp_lookup(ip: [u8; 4]) -> Option<[u8; 6]> {
    let t = ARP_TABLE.lock();
    t.iter().find(|e| e.ip == ip).map(|e| e.mac)
}

/// Добавить/обновить запись в ARP-таблице.
pub fn arp_remember(ip: [u8; 4], mac: [u8; 6]) {
    let mut t = ARP_TABLE.lock();
    if let Some(e) = t.iter_mut().find(|e| e.ip == ip) {
        e.mac = mac;
        return;
    }
    if t.len() >= ARP_TABLE_MAX {
        t.remove(0);
    }
    t.push(ArpEntry { ip, mac });
}

// ---------- Checksum ----------

fn ones_complement_sum(data: &[u8]) -> u32 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    sum
}

fn fold_checksum(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

// ---------- Ethernet ----------

fn build_eth_header(dst_mac: [u8; 6], src_mac: [u8; 6], eth_type: u16) -> [u8; 14] {
    let mut h = [0u8; 14];
    h[0..6].copy_from_slice(&dst_mac);
    h[6..12].copy_from_slice(&src_mac);
    h[12..14].copy_from_slice(&eth_type.to_be_bytes());
    h
}

/// Отправить Ethernet-кадр с заданным ethertype и payload.
pub fn send_eth(dst_mac: [u8; 6], eth_type: u16, payload: &[u8]) -> bool {
    let src_mac = match crate::e1000::mac() {
        Some(m) => m,
        None => return false,
    };
    let mut frame: Vec<u8> = Vec::with_capacity(14 + payload.len());
    frame.extend_from_slice(&build_eth_header(dst_mac, src_mac, eth_type));
    frame.extend_from_slice(payload);
    crate::e1000::send(&frame)
}

// ---------- ARP ----------

pub fn send_arp_request(target_ip: [u8; 4]) -> bool {
    let src_mac = match crate::e1000::mac() {
        Some(m) => m,
        None => return false,
    };
    let mut arp = [0u8; 28];
    arp[0..2].copy_from_slice(&1u16.to_be_bytes());      // htype = Ethernet
    arp[2..4].copy_from_slice(&ETH_TYPE_IPV4.to_be_bytes()); // ptype
    arp[4] = 6;                                          // hlen
    arp[5] = 4;                                          // plen
    arp[6..8].copy_from_slice(&1u16.to_be_bytes());      // oper = request
    arp[8..14].copy_from_slice(&src_mac);                // sha
    arp[14..18].copy_from_slice(&OUR_IP);                // spa
    arp[18..24].copy_from_slice(&[0u8; 6]);              // tha
    arp[24..28].copy_from_slice(&target_ip);             // tpa
    send_eth(BROADCAST_MAC, ETH_TYPE_ARP, &arp)
}

fn handle_arp(payload: &[u8]) {
    if payload.len() < 28 { return; }
    let oper = u16::from_be_bytes([payload[6], payload[7]]);
    if oper != 2 { return; } // не reply
    let mut sender_mac = [0u8; 6];
    sender_mac.copy_from_slice(&payload[8..14]);
    let mut sender_ip = [0u8; 4];
    sender_ip.copy_from_slice(&payload[14..18]);
    arp_remember(sender_ip, sender_mac);
    crate::log_debug!(
        "[net] ARP reply: {}.{}.{}.{} is {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        sender_ip[0], sender_ip[1], sender_ip[2], sender_ip[3],
        sender_mac[0], sender_mac[1], sender_mac[2],
        sender_mac[3], sender_mac[4], sender_mac[5]
    );
}

// ---------- IPv4 ----------

fn build_ipv4_header(src: [u8; 4], dst: [u8; 4], proto: u8, payload_len: usize) -> [u8; 20] {
    let total_len = (20 + payload_len) as u16;
    let id = IP_ID.fetch_add(1, Ordering::Relaxed);

    let mut h = [0u8; 20];
    h[0] = 0x45;                     // версия 4, IHL 5 (20 байт)
    h[1] = 0;                        // tos
    h[2..4].copy_from_slice(&total_len.to_be_bytes());
    h[4..6].copy_from_slice(&id.to_be_bytes());
    h[6..8].copy_from_slice(&0u16.to_be_bytes());  // flags/frag
    h[8] = 64;                       // TTL
    h[9] = proto;
    h[10..12].copy_from_slice(&0u16.to_be_bytes()); // csum — заполним
    h[12..16].copy_from_slice(&src);
    h[16..20].copy_from_slice(&dst);

    let csum = fold_checksum(ones_complement_sum(&h));
    h[10..12].copy_from_slice(&csum.to_be_bytes());
    h
}

// ---------- ICMP ----------

pub const ICMP_ECHO_REQUEST: u8 = 8;
pub const ICMP_ECHO_REPLY:   u8 = 0;

/// Отправить ICMP echo request по IP. Возвращает true, если удалось
/// отправить кадр (не значит, что получен ответ).
pub fn send_icmp_echo(dst_ip: [u8; 4], seq: u16, data: &[u8]) -> bool {
    let dst_mac = match arp_lookup(dst_ip) {
        Some(m) => m,
        None => return false,
    };
    let src_mac = match crate::e1000::mac() {
        Some(m) => m,
        None => return false,
    };

    // ICMP: type(1) code(1) csum(2) id(2) seq(2) data
    let icmp_len = 8 + data.len();
    let mut icmp: Vec<u8> = Vec::with_capacity(icmp_len);
    icmp.push(ICMP_ECHO_REQUEST);
    icmp.push(0);
    icmp.push(0);
    icmp.push(0);            // csum placeholder
    icmp.extend_from_slice(&0x4242u16.to_be_bytes()); // id
    icmp.extend_from_slice(&seq.to_be_bytes());
    icmp.extend_from_slice(data);

    let csum = fold_checksum(ones_complement_sum(&icmp));
    icmp[2..4].copy_from_slice(&csum.to_be_bytes());

    let ip_header = build_ipv4_header(OUR_IP, dst_ip, 1, icmp.len());
    let mut ip_packet: Vec<u8> = Vec::with_capacity(20 + icmp.len());
    ip_packet.extend_from_slice(&ip_header);
    ip_packet.extend_from_slice(&icmp);

    let _ = src_mac;
    send_eth(dst_mac, ETH_TYPE_IPV4, &ip_packet)
}

// ---------- Приём ----------

/// Обработка принятого Ethernet-кадра. Возвращает ICMP reply, если
/// пришёл echo reply (для user ping).
pub fn handle_frame(frame: &[u8]) -> Option<IcmpReply> {
    if frame.len() < 14 { return None; }
    let eth_type = u16::from_be_bytes([frame[12], frame[13]]);
    let payload = &frame[14..];

    match eth_type {
        ETH_TYPE_ARP => {
            handle_arp(payload);
            None
        }
        ETH_TYPE_IPV4 => handle_ipv4(payload),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct IcmpReply {
    pub src_ip: [u8; 4],
    pub seq: u16,
    pub id: u16,
    pub data_len: usize,
}

fn handle_ipv4(payload: &[u8]) -> Option<IcmpReply> {
    if payload.len() < 20 { return None; }
    let ihl = (payload[0] & 0x0F) as usize * 4;
    if ihl < 20 || ihl > payload.len() { return None; }
    let proto = payload[9];
    let src_ip = [payload[12], payload[13], payload[14], payload[15]];

    if proto != 1 { return None; } // не ICMP
    let icmp = &payload[ihl..];
    if icmp.len() < 8 { return None; }
    let icmp_type = icmp[0];
    if icmp_type != ICMP_ECHO_REPLY { return None; }
    let id = u16::from_be_bytes([icmp[4], icmp[5]]);
    let seq = u16::from_be_bytes([icmp[6], icmp[7]]);
    let data_len = icmp.len() - 8;
    Some(IcmpReply { src_ip, seq, id, data_len })
}

/// Небольшая обёртка для пользователя: подождать ARP и ICMP reply
/// с таймаутом. Возвращает `Some(reply)` или `None`.
pub fn wait_icmp_reply(
    timeout_ms: u64,
    want_seq: u16,
    want_id: u16,
) -> Option<IcmpReply> {
    let start = crate::interrupts::ticks();
    let timeout_ticks = (timeout_ms * 18 / 1000).max(1);
    let mut frame_buf = [0u8; 2048];
    loop {
        while let Some(n) = crate::e1000::recv(&mut frame_buf) {
            if let Some(r) = handle_frame(&frame_buf[..n]) {
                if r.seq == want_seq && r.id == want_id {
                    return Some(r);
                }
            }
        }
        let now = crate::interrupts::ticks();
        if now.saturating_sub(start) >= timeout_ticks {
            return None;
        }
        crate::sched::sleep_ms(5);
    }
}

/// Простой ping одного адреса. Возвращает Round-Trip Time в миллисекундах
/// или `None` при таймауте.
pub fn ping(dst_ip: [u8; 4], timeout_ms: u64) -> Option<u64> {
    // 1. ARP, если ещё не знаем MAC.
    if arp_lookup(dst_ip).is_none() {
        let _ = send_arp_request(dst_ip);
        // Ждём ARP reply.
        let start = crate::interrupts::ticks();
        let timeout_ticks = (1000u64 * 18 / 1000).max(1); // 1 сек
        let mut frame_buf = [0u8; 2048];
        while arp_lookup(dst_ip).is_none() {
            while let Some(n) = crate::e1000::recv(&mut frame_buf) {
                let _ = handle_frame(&frame_buf[..n]);
            }
            if crate::interrupts::ticks().saturating_sub(start) >= timeout_ticks {
                crate::log_warn!(
                    "[net] ARP timeout for {}.{}.{}.{}",
                    dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3]
                );
                return None;
            }
            crate::sched::sleep_ms(5);
        }
    }

    // 2. ICMP echo.
    let seq: u16 = (crate::interrupts::ticks() & 0xFFFF) as u16;
    let id: u16 = 0x4242;
    let payload = b"RustOS ping!";
    let t0 = crate::interrupts::ticks();
    if !send_icmp_echo(dst_ip, seq, payload) {
        return None;
    }
    let reply = wait_icmp_reply(timeout_ms, seq, id)?;
    let dt_ticks = crate::interrupts::ticks().saturating_sub(t0);
    let rtt_ms = dt_ticks * 1000 / 18;
    crate::log_info!(
        "[net] ICMP reply from {}.{}.{}.{} seq={} rtt={}ms data={}",
        reply.src_ip[0], reply.src_ip[1], reply.src_ip[2], reply.src_ip[3],
        reply.seq, rtt_ms, reply.data_len
    );
    Some(rtt_ms)
}

pub fn gateway_ip() -> [u8; 4] { GATEWAY_IP }
pub fn our_ip() -> [u8; 4] { OUR_IP }