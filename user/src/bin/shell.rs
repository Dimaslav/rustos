#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use userlib::*;

const LINE_MAX: usize = 256;
const CWD_MAX: usize = 256;
const SAVED_STDIN: u32 = 100;
const SAVED_STDOUT: u32 = 101;
const HISTORY_MAX: usize = 32;

static mut LINE: [u8; LINE_MAX] = [0u8; LINE_MAX];
static mut LEN: usize = 0;

static mut CWD: [u8; CWD_MAX] = [0u8; CWD_MAX];
static mut CWD_LEN: usize = 0;

static mut BG_PIDS: Vec<u64> = Vec::new();

static mut HISTORY: Vec<String> = Vec::new();
static mut HISTORY_IDX: isize = -1;

fn cwd() -> &'static str {
    unsafe { core::str::from_utf8(&CWD[..CWD_LEN]).unwrap_or("/") }
}

fn set_cwd(s: &[u8]) {
    unsafe {
        let n = s.len().min(CWD_MAX);
        CWD[..n].copy_from_slice(&s[..n]);
        CWD_LEN = n;
    }
}

fn prompt() {
    write(b"rustos:");
    write(cwd().as_bytes());
    write(b"$ ");
}

unsafe fn replace_line(new_bytes: &[u8]) {
    for _ in 0..LEN {
        write(b"\x08 \x08");
    }
    let n = new_bytes.len().min(LINE_MAX);
    LINE[..n].copy_from_slice(&new_bytes[..n]);
    LEN = n;
    if n > 0 {
        write(&LINE[..n]);
    }
}

// ---------- Парсеры ----------

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut qchar = ' ';
    for c in s.chars() {
        if in_quote {
            if c == qchar { in_quote = false; } else { cur.push(c); }
        } else if c == '"' || c == '\'' {
            in_quote = true; qchar = c;
        } else if c == ' ' || c == '\t' {
            if !cur.is_empty() { out.push(core::mem::take(&mut cur)); }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() { out.push(cur); }
    out
}

fn split_pipes(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut qchar = ' ';
    for c in s.chars() {
        if in_quote {
            cur.push(c);
            if c == qchar { in_quote = false; }
        } else if c == '"' || c == '\'' {
            in_quote = true; qchar = c; cur.push(c);
        } else if c == '|' {
            out.push(core::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

// ---------- Пути ----------

fn normalize(path: &str) -> String {
    if path.starts_with('/') || path.starts_with("C:") { return path.to_string(); }
    let c = cwd();
    if c == "/" { format!("/{}", path) } else { format!("{}/{}", c, path) }
}

fn canonicalize(path: &str) -> String {
    if path.starts_with("C:") { return path.to_string(); }
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        if p.is_empty() || p == "." { continue; }
        if p == ".." { let _ = parts.pop(); } else { parts.push(p); }
    }
    if parts.is_empty() { "/".to_string() }
    else {
        let mut s = String::new();
        for p in &parts { s.push('/'); s.push_str(p); }
        s
    }
}

// ---------- Builtins ----------

fn cmd_help() {
    write(b"commands:\n");
    write(b"  help             - this message\n");
    write(b"  exit             - exit shell\n");
    write(b"  cd [path]        - change directory\n");
    write(b"  pwd              - print working directory\n");
    write(b"  ls [path]        - list directory\n");
    write(b"  cat FILE         - print file\n");
    write(b"  cp SRC DST       - copy file\n");
    write(b"  wc [FILE...]     - count lines/words/bytes\n");
    write(b"  head [-n N] [FILE] - first N lines (default 10)\n");
    write(b"  mkdir PATH       - create directory\n");
    write(b"  rm PATH          - remove file/dir\n");
    write(b"  echo ARGS...     - print args\n");
    write(b"  ps               - list processes\n");
    write(b"  dmesg            - kernel log (/proc/kmsg)\n");
    write(b"  kill PID...      - terminate processes\n");
    write(b"  uname [-r]       - system info\n");
    write(b"  which PROG...    - locate a program\n");
    write(b"  env              - print environment\n");
    write(b"  viewer [FILE]    - view PNG/BMP (no arg: gradient)\n");
    write(b"  play FILE.wav    - play WAV via AC'97\n");
    write(b"  beep [FREQ] [MS] - play a tone via AC'97 (default 440/300)\n");
    write(b"  ping IP          - ICMP echo (e.g. ping 10.0.2.2)\n");
    write(b"  version          - system info\n");
    write(b"  calc             - open calculator\n");
    write(b"  repl PROG        - replace shell with PROG (exec)\n");
    write(b"  fork             - test fork\n");
    write(b"  crash            - trigger page fault (test init restart)\n");
    write(b"  jobs             - list background jobs\n");
    write(b"  wait             - wait for all background jobs\n");
    write(b"  cmd &            - run cmd in background\n");
    write(b"  cmd1 | cmd2      - pipe\n");
    write(b"  cmd > file       - write output\n");
    write(b"  cmd < file       - read input\n");
    write(b"  Up/Down arrows   - history\n");
}

fn cmd_version() {
    let s = format!("Rust OS 0.9 / pid={}\n", getpid());
    write(s.as_bytes());
}

fn cmd_cd(args: &[String]) {
    let target = if args.len() >= 2 { args[1].clone() } else { String::from("/") };
    let abs = canonicalize(&normalize(&target));
    match stat(&abs) {
        Some(s) if s.is_dir != 0 => set_cwd(abs.as_bytes()),
        _ => {
            write(b"cd: no such directory: ");
            write(abs.as_bytes());
            write(b"\n");
        }
    }
}

fn cmd_pwd() { write(cwd().as_bytes()); write(b"\n"); }

fn cmd_mkdir(args: &[String]) {
    if args.len() < 2 { write(b"usage: mkdir PATH\n"); return; }
    let abs = canonicalize(&normalize(&args[1]));
    if !mkdir(&abs) { write(b"mkdir: failed\n"); }
}

fn cmd_rm(args: &[String]) {
    if args.len() < 2 { write(b"usage: rm PATH\n"); return; }
    let abs = canonicalize(&normalize(&args[1]));
    if !unlink(&abs) { write(b"rm: failed\n"); }
}

fn cmd_echo(args: &[String]) {
    for (i, a) in args.iter().enumerate().skip(1) {
        if i > 1 { write(b" "); }
        write(a.as_bytes());
    }
    write(b"\n");
}

fn cmd_repl(args: &[String]) {
    if args.len() < 2 { write(b"usage: repl PROG\n"); return; }
    write(b"replacing shell with ");
    write(args[1].as_bytes());
    write(b"...\n");
    let _ = exec_replace(&args[1]);
    write(b"exec_replace failed\n");
}

fn cmd_fork() {
    write(b"[fork] calling fork()...\n");
    let pid = fork();
    if pid == u64::MAX {
        write(b"[fork] fork() failed\n");
        return;
    }
    if pid == 0 {
        let s = format!("[child]  hello, my pid={}\n", getpid());
        write(s.as_bytes());
        sleep_ms(50);
        write(b"[child]  exiting with code 7\n");
        exit(7);
    } else {
        let s = format!("[parent] hello, my pid={}, child pid={}\n", getpid(), pid);
        write(s.as_bytes());
        match wait(pid) {
            Some(code) => {
                let s = format!("[parent] child exited with code {}\n", code);
                write(s.as_bytes());
            }
            None => write(b"[parent] wait() failed\n"),
        }
    }
}

fn cmd_jobs() {
    unsafe {
        if BG_PIDS.is_empty() {
            write(b"(no background jobs)\n");
            return;
        }
        for (i, pid) in BG_PIDS.iter().enumerate() {
            let s = format!("[{}] pid {}\n", i + 1, pid);
            write(s.as_bytes());
        }
    }
}

fn cmd_wait() {
    unsafe {
        let pids = core::mem::take(&mut BG_PIDS);
        for pid in pids {
            let _ = wait(pid);
        }
        write(b"[wait] all children reaped\n");
    }
}

fn cmd_crash() {
    write(b"[shell] crashing on purpose...\n");
    unsafe {
        core::ptr::write_volatile(0x1000 as *mut u8, 0xAB);
    }
}

fn cmd_beep(args: &[String]) {
    let freq: u32 = if args.len() >= 2 {
        args[1].parse().unwrap_or(440)
    } else {
        440
    };
    let ms: u32 = if args.len() >= 3 {
        args[2].parse().unwrap_or(300)
    } else {
        300
    };
    let msg = format!("beep: {} Hz, {} ms\n", freq, ms);
    write(msg.as_bytes());
    beep(freq, ms);
    write(b"beep: done\n");
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "help" | "exit" | "cd" | "pwd" | "mkdir" | "rm" | "echo"
            | "version" | "repl" | "fork" | "jobs" | "wait" | "crash"
            | "beep"
    )
}

fn run_builtin(args: &[String]) {
    match args[0].as_str() {
        "help"    => cmd_help(),
        "version" => cmd_version(),
        "exit"    => { set_focus(false); exit(0); }
        "cd"      => cmd_cd(args),
        "pwd"     => cmd_pwd(),
        "mkdir"   => cmd_mkdir(args),
        "rm"      => cmd_rm(args),
        "echo"    => cmd_echo(args),
        "repl"    => cmd_repl(args),
        "fork"    => cmd_fork(),
        "jobs"    => cmd_jobs(),
        "wait"    => cmd_wait(),
        "crash"   => cmd_crash(),
        "beep"    => cmd_beep(args),
        _ => {}
    }
}

// ---------- Редиректы и pipes ----------

struct Segment {
    args: Vec<String>,
    input: Option<String>,
    output: Option<(String, bool)>,
}

fn parse_segment(s: &str) -> Option<Segment> {
    let tokens = split_args(s);
    let mut args = Vec::new();
    let mut input: Option<String> = None;
    let mut output: Option<(String, bool)> = None;
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "<" => { if i + 1 >= tokens.len() { return None; } input = Some(tokens[i+1].clone()); i += 2; }
            ">" => { if i + 1 >= tokens.len() { return None; } output = Some((tokens[i+1].clone(), false)); i += 2; }
            ">>" => { if i + 1 >= tokens.len() { return None; } output = Some((tokens[i+1].clone(), true)); i += 2; }
            _ => { args.push(tokens[i].clone()); i += 1; }
        }
    }
    if args.is_empty() { return None; }
    Some(Segment { args, input, output })
}

fn spawn_external(args: &[String]) -> Option<u64> {
    let mut owned: Vec<String> = Vec::with_capacity(args.len());
    for (i, a) in args.iter().enumerate() {
        if i == 0 {
            owned.push(a.clone());
        } else if matches!(args[0].as_str(),
            "cat" | "ls" | "cp" | "wc" | "head" | "dmesg" | "viewer" | "play")
        {
            owned.push(canonicalize(&normalize(a)));
        } else {
            owned.push(a.clone());
        }
    }
    let argv: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    let child = exec_argv(&args[0], &argv);
    if child == u64::MAX { None } else { Some(child) }
}

fn run_segments(segments: &[Segment]) {
    let mut children: Vec<u64> = Vec::new();
    let mut next_stdin: Option<u32> = None;

    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;

        let (pipe_read, pipe_write) = if is_last { (None, None) } else {
            match pipe() {
                Some((r, w)) => (Some(r), Some(w)),
                None => { write(b"pipe: failed\n"); return; }
            }
        };

        let file_stdin: Option<u32> = match &seg.input {
            Some(f) => {
                let abs = canonicalize(&normalize(f));
                let fd = open(&abs);
                if fd == u64::MAX { write(b"cannot open\n"); return; }
                Some(fd as u32)
            }
            None => None,
        };
        let file_stdout: Option<u32> = match &seg.output {
            Some((f, append)) => {
                let abs = canonicalize(&normalize(f));
                let fd = open_write(&abs, *append);
                if fd == u64::MAX { write(b"cannot create\n"); return; }
                Some(fd as u32)
            }
            None => None,
        };

        let use_stdin = file_stdin.or(next_stdin);
        let use_stdout = file_stdout.or(pipe_write);

        let do_save_in = use_stdin.is_some();
        let do_save_out = use_stdout.is_some();
        if do_save_in { dup2(0, SAVED_STDIN); }
        if do_save_out { dup2(1, SAVED_STDOUT); }
        if let Some(r) = use_stdin { dup2(r, 0); }
        if let Some(w) = use_stdout { dup2(w, 1); }

        let child = if is_builtin(&seg.args[0]) {
            run_builtin(&seg.args);
            None
        } else {
            spawn_external(&seg.args)
        };

        if do_save_in { dup2(SAVED_STDIN, 0); close(SAVED_STDIN as u64); }
        if do_save_out { dup2(SAVED_STDOUT, 1); close(SAVED_STDOUT as u64); }
        if let Some(r) = use_stdin { close(r as u64); }
        if let Some(w) = use_stdout { close(w as u64); }

        next_stdin = pipe_read;

        if let Some(c) = child {
            children.push(c);
        } else if !is_builtin(&seg.args[0]) {
            write(b"command not found: ");
            write(seg.args[0].as_bytes());
            write(b"\n");
            return;
        }
    }

    for c in children { let _ = wait(c); }
}

fn run_background(seg: &Segment) {
    if seg.input.is_some() || seg.output.is_some() {
        write(b"bg: redirects not supported\n");
        return;
    }
    if is_builtin(&seg.args[0]) {
        write(b"bg: builtins cannot run in background\n");
        return;
    }

    let pid = fork();
    if pid == u64::MAX {
        write(b"bg: fork failed\n");
        return;
    }
    if pid == 0 {
        let _ = exec_replace(&seg.args[0]);
        write(b"bg: exec_replace failed\n");
        exit(1);
    }

    let s = format!("[bg] pid {} — {}\n", pid, seg.args[0]);
    write(s.as_bytes());
    unsafe { BG_PIDS.push(pid); }
}

fn execute(line: &str) {
    let mut trimmed = line.trim();
    if trimmed.is_empty() { return; }

    let background = trimmed.ends_with('&');
    if background {
        trimmed = trimmed.trim_end_matches('&').trim_end();
    }
    if trimmed.is_empty() { return; }

    let parts = split_pipes(trimmed);
    let mut segments: Vec<Segment> = Vec::new();
    for part in &parts {
        match parse_segment(part) {
            Some(s) => segments.push(s),
            None => { write(b"syntax error\n"); return; }
        }
    }
    if segments.is_empty() { return; }

    if background {
        if segments.len() > 1 {
            write(b"bg: pipe not supported in background\n");
            return;
        }
        run_background(&segments[0]);
    } else {
        run_segments(&segments);
    }
}

#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8) -> ! {
    write(b"[shell] started, type 'help'\n");
    set_cwd(b"/");
    set_focus(true);
    prompt();

    loop {
        let k = read_key();
        if k == u64::MAX { sleep_ms(30); continue; }
        let b = k as u8;
        unsafe {
            match b {
                0x0A => {
                    write(b"\n");
                    if LEN > 0 {
                        let s = core::str::from_utf8(&LINE[..LEN])
                            .unwrap_or("").to_string();
                        let is_dup = HISTORY.last()
                            .map(|h| h == &s).unwrap_or(false);
                        if !is_dup {
                            if HISTORY.len() >= HISTORY_MAX {
                                HISTORY.remove(0);
                            }
                            HISTORY.push(s.clone());
                        }
                        execute(&s);
                    }
                    HISTORY_IDX = -1;
                    LEN = 0;
                    prompt();
                }
                0x08 => {
                    if LEN > 0 { LEN -= 1; write(b"\x08 \x08"); }
                }
                0x80 => {
                    if !HISTORY.is_empty() {
                        let cur = if HISTORY_IDX < 0 {
                            HISTORY.len() as isize
                        } else {
                            HISTORY_IDX
                        };
                        let new = (cur - 1).max(0);
                        HISTORY_IDX = new;
                        let line = HISTORY[new as usize].clone();
                        replace_line(line.as_bytes());
                    }
                }
                0x81 => {
                    if !HISTORY.is_empty() && HISTORY_IDX >= 0 {
                        let cur = HISTORY_IDX as usize;
                        if cur + 1 >= HISTORY.len() {
                            HISTORY_IDX = -1;
                            replace_line(b"");
                        } else {
                            HISTORY_IDX = (cur + 1) as isize;
                            let line = HISTORY[HISTORY_IDX as usize].clone();
                            replace_line(line.as_bytes());
                        }
                    }
                }
                c if (0x20..0x7F).contains(&c) => {
                    if LEN < LINE_MAX {
                        LINE[LEN] = c;
                        LEN += 1;
                        let buf = [c];
                        write(&buf);
                    }
                }
                _ => {}
            }
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }