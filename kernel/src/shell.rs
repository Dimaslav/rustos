use alloc::string::String;
use x86_64::instructions::interrupts;

use crate::framebuffer;
use crate::keyboard::pop_char;
use crate::{print, println};

pub fn run() -> ! {
    let mut buffer = String::new();

    println!("Type 'help' for a list of commands.");
    print!("> ");

    loop {
        if let Some(c) = pop_char() {
            match c {
                b'\n' => {
                    println!();
                    execute(&buffer);
                    buffer.clear();
                    print!("> ");
                }
                0x08 => {
                    if buffer.pop().is_some() {
                        framebuffer::backspace();
                    }
                }
                b'\t' => {
                    // На будущее
                }
                c if (0x20..0x7F).contains(&c) => {
                    buffer.push(c as char);
                    print!("{}", c as char);
                }
                _ => {}
            }
        } else {
            interrupts::enable_and_hlt();
        }
    }
}

fn execute(line: &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    let mut parts = trimmed.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("");

    match cmd {
        "help" => {
            println!("Available commands:");
            println!("  help        -- show this message");
            println!("  clear       -- clear screen");
            println!("  echo <text> -- print text");
            println!("  about       -- OS info");
            println!("  heap        -- show heap usage");
        }
        "clear" => {
            framebuffer::clear();
        }
        "echo" => {
            println!("{}", arg);
        }
        "about" => {
            println!("Rust OS v0.1.0");
            println!("Custom kernel, framebuffer + interrupts + heap.");
        }
        "heap" => {
            use crate::allocator::ALLOCATOR;
            let used = ALLOCATOR.lock().used();
            let free = ALLOCATOR.lock().free();
            println!("Heap used: {} bytes", used);
            println!("Heap free: {} bytes", free);
        }
        _ => {
            println!("Unknown command: {}", cmd);
            println!("Type 'help' for a list of commands.");
        }
    }
}