//! Pipe — ring buffer между процессами.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use spin::Mutex;

pub const PIPE_BUF_SIZE: usize = 4096;
pub const MAX_PIPES: usize = 64;

pub struct Pipe {
    pub id: u32,
    pub buf: VecDeque<u8>,
    pub readers: u32,
    pub writers: u32,
    pub alive: bool,
}

impl Pipe {
    fn new(id: u32) -> Self {
        Self {
            id,
            buf: VecDeque::with_capacity(PIPE_BUF_SIZE),
            readers: 1,
            writers: 1,
            alive: true,
        }
    }
}

static PIPES: Mutex<Vec<Pipe>> = Mutex::new(Vec::new());
static NEXT_PIPE: Mutex<u32> = Mutex::new(0);

pub fn create() -> Option<u32> {
    let mut p = PIPES.lock();
    if p.len() >= MAX_PIPES { return None; }
    let mut n = NEXT_PIPE.lock();
    let id = *n;
    *n = n.wrapping_add(1);
    p.push(Pipe::new(id));
    Some(id)
}

pub fn read(pipe_id: u32, buf: &mut [u8]) -> Option<usize> {
    let mut p = PIPES.lock();
    let pipe = p.iter_mut().find(|x| x.id == pipe_id && x.alive)?;
    if pipe.buf.is_empty() {
        if pipe.writers == 0 {
            return Some(0);
        }
        return None; // callер сделает yield
    }
    let n = buf.len().min(pipe.buf.len());
    for i in 0..n {
        buf[i] = pipe.buf.pop_front().unwrap();
    }
    Some(n)
}

pub fn write(pipe_id: u32, data: &[u8]) -> Option<usize> {
    let mut p = PIPES.lock();
    let pipe = p.iter_mut().find(|x| x.id == pipe_id && x.alive)?;
    if pipe.readers == 0 { return Some(0); }
    let space = PIPE_BUF_SIZE - pipe.buf.len();
    if space == 0 { return None; }
    let n = data.len().min(space);
    for &b in &data[..n] {
        pipe.buf.push_back(b);
    }
    Some(n)
}

pub fn add_reader(pipe_id: u32) -> bool {
    let mut p = PIPES.lock();
    if let Some(pipe) = p.iter_mut().find(|x| x.id == pipe_id && x.alive) {
        pipe.readers += 1;
        true
    } else { false }
}

pub fn add_writer(pipe_id: u32) -> bool {
    let mut p = PIPES.lock();
    if let Some(pipe) = p.iter_mut().find(|x| x.id == pipe_id && x.alive) {
        pipe.writers += 1;
        true
    } else { false }
}

pub fn close_reader(pipe_id: u32) {
    let mut p = PIPES.lock();
    if let Some(pipe) = p.iter_mut().find(|x| x.id == pipe_id && x.alive) {
        pipe.readers = pipe.readers.saturating_sub(1);
        if pipe.readers == 0 && pipe.writers == 0 {
            pipe.alive = false;
        }
    }
}

pub fn close_writer(pipe_id: u32) {
    let mut p = PIPES.lock();
    if let Some(pipe) = p.iter_mut().find(|x| x.id == pipe_id && x.alive) {
        pipe.writers = pipe.writers.saturating_sub(1);
        if pipe.readers == 0 && pipe.writers == 0 {
            pipe.alive = false;
        }
    }
}