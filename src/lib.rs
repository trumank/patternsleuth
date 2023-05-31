#![feature(portable_simd)]

pub mod patterns;
pub mod scanner;

use anyhow::{bail, Result};

use patterns::PatternID;

#[derive(Debug)]
enum PatternMode {
    Direct,
    Relative32(isize),
}

#[derive(Debug)]
pub struct Pattern {
    sig: Vec<u8>,
    mask: Vec<u8>,
    mode: PatternMode,
}

impl Pattern {
    pub fn new(s: &str) -> Result<Self> {
        let mut sig = vec![];
        let mut mask = vec![];
        let mut mode = PatternMode::Direct;

        for w in s.split_whitespace() {
            if let Ok(b) = u8::from_str_radix(w, 16) {
                sig.push(b);
                mask.push(0xff);
                continue;
            } else if w == "??" {
                if sig.is_empty() {
                    bail!("first byte cannot be \"??\"");
                }
                sig.push(0);
                mask.push(0);
                continue;
            } else if let Some(r) = w.strip_prefix("R32") {
                if r.is_empty() {
                    mode = PatternMode::Relative32(sig.len() as isize);
                    continue;
                } else if let Ok(offset) = isize::from_str_radix(r, 16) {
                    mode = PatternMode::Relative32(sig.len() as isize + offset);
                    continue;
                }
            }
            bail!("bad pattern word \"{}\"", w);
        }

        Ok(Self { sig, mask, mode })
    }
    #[inline]
    fn is_match(&self, data: &[u8], index: usize) -> bool {
        for i in 0..self.mask.len() {
            if data[index + i] & self.mask[i] != self.sig[i] {
                return false;
            }
        }
        true
    }
    fn compute_result(&self, data: &[u8], base_address: usize, index: usize) -> usize {
        base_address
            + match self.mode {
                PatternMode::Direct => index,
                PatternMode::Relative32(offset) => {
                    let n = index.checked_add_signed(offset).unwrap();
                    n.checked_add_signed(
                        i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize
                    )
                    .unwrap()
                }
            }
    }
}

#[derive(Debug)]
pub struct Resolution {
    /// name of section pattern was found in
    pub section: String,
    /// intermediate addresses of interest before reaching the final address
    pub stages: Vec<usize>,
    /// final, fully resolved address
    pub address: Option<usize>,
}

pub struct PatternConfig {
    pub id: PatternID,
    pub section: Option<object::SectionKind>,
    pub pattern: Pattern,
}
impl PatternConfig {
    fn new(
        id: patterns::PatternID,
        section: Option<object::SectionKind>,
        pattern: Pattern,
    ) -> Self {
        Self {
            id,
            section,
            pattern,
        }
    }
}
