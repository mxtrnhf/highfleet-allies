//! Standalone microbenchmark, not a game/frame/FPS benchmark or regression suite.
//! Build: rustc --edition 2021 -O examples/font_color.rs -o /path/font-color-bench

#[path = "../src/font_color.rs"]
mod font_color;

use std::{arch::global_asm, hint::black_box, time::Instant};

// Original 1.163 instructions, with only labels/call displacement relocated.
// Both callees use Win64 ABI even when this benchmark runs on Linux.
global_asm!(
    ".text",
    ".global native_font_color",
    "native_font_color:",
    "mov [rsp + 8], rbx",
    "mov [rsp + 16], rsi",
    "push rdi",
    "sub rsp, 32",
    "mov esi, edx",
    "mov [rcx + 0x1020], edx",
    "lea rbx, [rcx + 8]",
    "mov edi, 256",
    "2:",
    "mov rcx, [rbx]",
    "test rcx, rcx",
    "jz 3f",
    "mov r8d, -1",
    "mov edx, esi",
    "call native_sprite_color",
    "3:",
    "add rbx, 8",
    "sub rdi, 1",
    "jnz 2b",
    "mov rbx, [rsp + 48]",
    "mov rsi, [rsp + 56]",
    "add rsp, 32",
    "pop rdi",
    "ret",
    "native_sprite_color:",
    "cmp r8d, -1",
    "je 4f",
    "movsxd rax, r8d",
    "imul r8, rax, 88",
    "mov [r8 + rcx + 24], edx",
    "ret",
    "4:",
    "mov [rcx + 0x120], edx",
    "mov [rcx + 0xc8], edx",
    "mov [rcx + 0x70], edx",
    "mov [rcx + 0x18], edx",
    "ret",
);

extern "win64" {
    fn native_font_color(font: *mut u8, color: u32);
}

// Alternative evaluated here only: always write, but inline the trivial setter.
unsafe extern "win64" fn unconditional_batch(font: *mut u8, color: u32) {
    unsafe {
        font.add(font_color::FONT_COLOR).cast::<u32>().write(color);
        for index in 0..font_color::GLYPH_COUNT {
            let glyph = font.add(8 + index * 8).cast::<*mut u8>().read();
            if !glyph.is_null() {
                for offset in font_color::CORNER_COLORS.into_iter().rev() {
                    glyph.add(offset).cast::<u32>().write(color);
                }
            }
        }
    }
}

type Setter = unsafe extern "win64" fn(*mut u8, u32);

struct Fixture {
    font: Box<[u64; 0x1030 / 8]>,
    glyphs: Vec<[u64; 0x1a8 / 8]>,
}

impl Fixture {
    fn new(count: usize) -> Self {
        let mut fixture = Self {
            font: Box::new([0; 0x1030 / 8]),
            glyphs: vec![[0x5a5a5a5a5a5a5a5a; 0x1a8 / 8]; count],
        };
        for (index, glyph) in fixture.glyphs.iter_mut().enumerate() {
            fixture.font[index + 1] = glyph.as_mut_ptr() as u64;
        }
        fixture
    }

    fn pointer(&mut self) -> *mut u8 {
        self.font.as_mut_ptr().cast()
    }

    fn fingerprint(&self) -> Vec<u64> {
        // Font pointer addresses differ between fixtures; compare all other bytes.
        let mut state = self.font.to_vec();
        state[1..257].fill(0);
        for glyph in &self.glyphs {
            state.extend_from_slice(glyph);
        }
        state
    }
}

fn equivalent(count: usize) -> bool {
    let mut native = Fixture::new(count);
    let mut candidate = Fixture::new(count);
    for color in [0, 0xffa8fc00, 0xffa8fc00, 0xffffffff, 0, 0] {
        // Simulate independent per-corner recoloring despite an unchanged dwCol.
        for fixture in [&mut native, &mut candidate] {
            if let Some(glyph) = fixture.glyphs.last_mut() {
                unsafe {
                    glyph
                        .as_mut_ptr()
                        .cast::<u8>()
                        .add(0xc8)
                        .cast::<u32>()
                        .write(0x12345678);
                }
            }
        }
        unsafe {
            native_font_color(native.pointer(), color);
            font_color::set_color(candidate.pointer(), color);
        }
        if native.fingerprint() != candidate.fingerprint() {
            return false;
        }
    }
    true
}

fn measure(setter: Setter, count: usize, alternating: bool, iterations: u32) -> f64 {
    let mut fixture = Fixture::new(count);
    let pointer = black_box(fixture.pointer());
    let start = Instant::now();
    for index in 0..iterations {
        let color = if alternating {
            0xffa8fc00 ^ (index & 1)
        } else {
            0xffa8fc00
        };
        unsafe { black_box(setter)(pointer, black_box(color)) };
    }
    let elapsed = start.elapsed();
    black_box(fixture.fingerprint());
    elapsed.as_nanos() as f64 / f64::from(iterations)
}

fn main() {
    for count in [0, 1, 128, 256] {
        if !equivalent(count) {
            eprintln!("Full-state comparison failed: glyph count {count}");
            std::process::exit(1);
        }
    }
    eprintln!("Full-state comparison passed; synthetic fixtures only, not in-game equivalence.");
    println!("glyphs,alternating,round,variant,ns_per_call");
    let variants: [(&str, Setter); 3] = [
        ("native", native_font_color),
        ("checked_batch", font_color::set_color),
        ("unconditional_batch", unconditional_batch),
    ];
    for count in [128, 256] {
        for alternating in [false, true] {
            for round in 0..7 {
                // Rotate ordering to reduce systematic thermal/order bias.
                for step in 0..3 {
                    let (name, setter) = variants[(round + step) % 3];
                    let nanos = measure(setter, count, alternating, 300_000);
                    println!("{count},{alternating},{round},{name},{nanos:.3}");
                }
            }
        }
    }
}
