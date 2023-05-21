#![feature(portable_simd)]

use anyhow::{bail, Result};
use std::simd::{SimdPartialEq, ToBitMask};

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

pub fn scan<'id, ID: Sync>(
    patterns: &[(&'id ID, &Pattern)],
    base_address: usize,
    data: &[u8],
) -> Vec<(&'id ID, usize)> {
    use rayon::prelude::*;

    const LANES: usize = 16;

    let max = patterns.iter().map(|(_id, p)| p.sig.len()).max().unwrap();

    let first: Vec<_> = patterns
        .iter()
        .map(|p| (p, std::simd::Simd::splat(p.1.sig[0])))
        .collect();

    // split data for simd
    // cut middle short such that even the longest pattern doesn't have to bounds check
    let (prefix, middle, _suffix) = data[0..data.len() - max].as_simd::<LANES>();
    let suffix = &data[prefix.len() + middle.len() * LANES..data.len()];

    let mut matches = vec![];

    // prefix
    for (id, p) in patterns {
        for i in 0..prefix.len().min(data.len().saturating_sub(p.sig.len() - 1)) {
            if p.is_match(data, i) {
                matches.push((*id, base_address + i));
            }
        }
    }

    // middle
    let batch_size = (middle.len()
        / std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::new(1).unwrap()))
    .max(1);
    let batches: Vec<_> = middle.chunks(batch_size).enumerate().collect();
    matches.append(
        &mut batches
            .par_iter()
            .map(|(index, batch)| {
                let mut matches = vec![];
                let offset = index * batch_size;

                for (i, chunk) in batch.iter().enumerate() {
                    for ((id, p), f) in &first {
                        let mut mask = f.simd_eq(*chunk).to_bitmask();

                        while mask != 0 {
                            let next = mask.trailing_zeros();
                            mask ^= 1 << next;

                            let j = prefix.len() + (offset + i) * LANES + next as usize;

                            if p.is_match(data, j) {
                                matches.push((*id, p.compute_result(data, base_address, j)));
                            }
                        }
                    }
                }
                matches
            })
            .flatten()
            .collect(),
    );

    // suffix
    let start = prefix.len() + middle.len() * LANES;
    for (id, p) in patterns {
        for i in start..start + suffix.len().saturating_sub(p.sig.len() - 1) {
            if p.is_match(data, i) {
                matches.push((*id, base_address + i));
            }
        }
    }

    matches
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_scan() {
        let patterns = [(&(), &Pattern::new("01").unwrap())];

        let len = 64;
        let lanes = 32;

        let data = vec![1; len + lanes];
        let matches: Vec<_> = (0..len).collect();

        for i in 0..lanes {
            let slice = &data[i..i + len];
            assert_eq!(
                matches,
                scan(&patterns, 0, slice)
                    .into_iter()
                    .map(|(_id, addr)| addr)
                    .collect::<Vec<_>>()
            );
        }

        let patterns = [(&(), &Pattern::new("01 02").unwrap())];

        // obtuse generator to test every combination of chunk boundaries
        let data: Vec<_> = std::iter::repeat([1, 2, 3]).take(32).flatten().collect();
        let matches: Vec<_> = (0..3)
            .map(|offset| (0..len / 3).map(|i| i * 3 + offset).collect::<Vec<_>>())
            .collect();

        for i in 0..(len - lanes) {
            let slice = &data[i..i + len];
            let res = scan(&patterns, 0, slice)
                .into_iter()
                .map(|(_id, addr)| addr)
                .collect::<Vec<_>>();
            assert_eq!(matches[(3 - (i % 3)) % 3], res);
        }
    }
}
