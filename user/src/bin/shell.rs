#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use userlib::*;

const LINE_MAX: usize = 128;
const SAVED_STDIN: u32 = 20;
const SAVED_STDOUT: u32 = 21;

static mut LINE: [u8; LINE_MAX] = [0u8; LINE_MAX];
static mut LEN: usize = 0;
static mut CWD: Option<String> = None;

fn cwd() -> String {
    unsafe {
        let p = &*core::ptr::addr_of!(CWD);
        p.clone().unwrap_or_else(|| String::from("/"))
    }
}

fn set_cwd(s: String) {
    unsafe {
        let p = &mut *core::ptr::addr_of_mut!(CWD);
        *p = Some(s);
    }
}

fn prompt() {
    write(b"rustos:");
    write(cwd().as_bytes());
    write(b"$ ");
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut qchar = ' ';
    for c in s.chars() {
        if in_quote {
            if c == qchar { in_quote = false; }
            else { cur.push(c); }
        } else if c == '"' || c == '\'' {
            in_quote = true;
            qchar = c;
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
            in_quote = true;
            qchar = c;
            cur.push(c);
        } else if c == '|' {
            out.push(core::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn normalize(path: &str) -> String {
    if path.starts_with('/') || path.starts_with("C:") {
        return path.to_string();
    }
    let c = cwd();
    if c == "/" { format!("/{}", path) }
    else { format!("{}/{}", c, path) }
}

fn canonicalize(path: &str) -> String {
    if path.starts_with("C:") {
        return path.to_string();
    }
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        if p.is_empty() || p == "." { continue; }
        if p == ".." {
            let _ = parts.pop();
        } else {
            parts.push(p);
        }
    }
    if parts.is_empty() {
        "/".to_string()
    } else {
        let mut s = String::new();
        for p in &parts {
            s.push('/');
            s.push_str(p);
        }
        s
    }
}

/// Разбор одного сегмента (между пайпами) с учётом `<`, `>`, `>>`.
/// Возвращает (args, input_file, output_file_append).
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
            "<" => {
                if i + 1 >= tokens.len() { return None; }
                input = Some(tokens[i + 1].clone());
                i += 2;
            }
            ">" => {
                if i + 1 >= tokens.len() { return None; }
                output = Some((tokens[i + 1].clone(), false));
                i += 2;
            }
            ">>" => {
                if i + 1 >= tokens.len() { return None; }
                output = Some((tokens[i + 1].clone(), true));
                i += 2;
            }
            _ => {
                args.push(tokens[i].clone());
                i += 1;
            }
        }
    }
    if args.is_empty() { return None; }
    Some(Segment { args, input, output })
}

fn cmd_help() {
    write(b"commands:\n");
    write(b"  help                - this message\n");
    write(b"  exit                - exit shell\n");
    write(b"  cd [path]           - change directory\n");
    write(b"  pwd                 - print working directory\n");
    write(b"  ls [path]           - list directory\n");
    write(b"  cat FILE            - print file (or stdin if no args)\n");
    write(b"  mkdir PATH          - create directory\n");
    write(b"  rm PATH             - remove file/dir\n");
    write(b"  echo ARGS...        - print args\n");
    write(b"  ps                  - list processes\n");
    write(b"  calc                - open calculator\n");
    write(b"  worker              - spawn worker\n");
    write(b"  cmd1 | cmd2         - pipe\n");
    write(b"  cmd > file          - write output to file\n");
    write(b"  cmd >> file         - append output\n");
    write(b"  cmd < file          - read input from file\n");
}

fn cmd_cd(args: &[String]) {
    let target = if args.len() >= 2 { args[1].clone() } else { String::from("/") };
    let abs = canonicalize(&normalize(&target));
    match stat(&abs) {
        Some(s) if s.is_dir != 0 => set_cwd(abs),
        _ => {
            write(b"cd: no such directory: ");
            write(abs.as_bytes());
            write(b"\n");
        }
    }
}

fn cmd_pwd() {
    let c = cwd();
    write(c.as_bytes());
    write(b"\n");
}

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

fn is_builtin(name: &str) -> bool {
    matches!(name, "help" | "exit" | "cd" | "pwd" | "mkdir" | "rm" | "echo")
}

fn run_builtin(args: &[String]) {
    match args[0].as_str() {
        "help" => cmd_help(),
        "exit" => { set_focus(false); exit(0); }
        "cd" => cmd_cd(args),
        "pwd" => cmd_pwd(),
        "mkdir" => cmd_mkdir(args),
        "rm" => cmd_rm(args),
        "echo" => cmd_echo(args),
        _ => {}
    }
}

fn spawn_external(args: &[String]) -> Option<u64> {
    let mut owned: Vec<String> = Vec::with_capacity(args.len());
    for (i, a) in args.iter().enumerate() {
        if i == 0 {
            owned.push(a.clone());
        } else if args[0] == "cat" || args[0] == "ls" {
            owned.push(canonicalize(&normalize(a)));
        } else {
            owned.push(a.clone());
        }
    }
    let argv: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    let child = exec_argv(&args[0], &argv);
    if child == u64::MAX { None } else { Some(child) }
}

/// Обработать всю цепочку сегментов (пайпов) с редиректами.
fn run_segments(segments: &[Segment]) {
    let mut children: Vec<u64> = Vec::new();
    let mut next_stdin: Option<u32> = None;

    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;

        // 1. Pipe для связи со следующей командой.
        let (pipe_read, pipe_write) = if is_last {
            (None, None)
        } else {
            match pipe() {
                Some((r, w)) => (Some(r), Some(w)),
                None => { write(b"pipe: failed\n"); return; }
            }
        };

        // 2. Открываем файлы редиректов.
        let file_stdin: Option<u32> = match &seg.input {
            Some(f) => {
                let abs = canonicalize(&normalize(f));
                let fd = open(&abs);
                if fd == u64::MAX {
                    write(b"cannot open: ");
                    write(abs.as_bytes());
                    write(b"\n");
                    return;
                }
                Some(fd as u32)
            }
            None => None,
        };
        let file_stdout: Option<u32> = match &seg.output {
            Some((f, append)) => {
                let abs = canonicalize(&normalize(f));
                let fd = open_write(&abs, *append);
                if fd == u64::MAX {
                    write(b"cannot create: ");
                    write(abs.as_bytes());
                    write(b"\n");
                    return;
                }
                Some(fd as u32)
            }
            None => None,
        };

        // 3. Определяем итоговые fd 0/1.
        let use_stdin = file_stdin.or(next_stdin);
        let use_stdout = file_stdout.or(pipe_write);

        // 4. Сохраняем оригинальные 0/1, если будем подменять.
        let do_save_in = use_stdin.is_some();
        let do_save_out = use_stdout.is_some();
        if do_save_in { dup2(0, SAVED_STDIN); }
        if do_save_out { dup2(1, SAVED_STDOUT); }

        // 5. Подменяем.
        if let Some(r) = use_stdin { dup2(r, 0); }
        if let Some(w) = use_stdout { dup2(w, 1); }

        // 6. Запуск.
        let child = if is_builtin(&seg.args[0]) {
            run_builtin(&seg.args);
            None
        } else {
            spawn_external(&seg.args)
        };

        // 7. Восстанавливаем.
        if do_save_in { dup2(SAVED_STDIN, 0); close(SAVED_STDIN as u64); }
        if do_save_out { dup2(SAVED_STDOUT, 1); close(SAVED_STDOUT as u64); }

        // 8. Закрываем в родителе лишние fd.
        if let Some(r) = use_stdin { close(r as u64); }
        if let Some(w) = use_stdout { close(w as u64); }

        // 9. Запоминаем read_end для следующей команды.
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

fn execute(line: &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() { return; }

    let parts = split_pipes(trimmed);
    let mut segments: Vec<Segment> = Vec::new();
    for part in &parts {
        match parse_segment(part) {
            Some(s) => segments.push(s),
            None => {
                write(b"syntax error\n");
                return;
            }
        }
    }
    if segments.is_empty() { return; }

    run_segments(&segments);
}

#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8) -> ! {
    write(b"[shell] started, type 'help'\n");
    set_cwd(String::from("/"));
    set_focus(true);
    prompt();

    loop {
        let k = read_key();
        if k == u64::MAX {
            sleep_ms(30);
            continue;
        }
        let b = k as u8;
        unsafe {
            match b {
                0x0A => {
                    write(b"\n");
                    let line_bytes = &LINE[..LEN];
                    if let Ok(s) = core::str::from_utf8(line_bytes) {
                        let owned = s.to_string();
                        execute(&owned);
                    }
                    LEN = 0;
                    prompt();
                }
                0x08 => {
                    if LEN > 0 {
                        LEN -= 1;
                        write(b"\x08 \x08");
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