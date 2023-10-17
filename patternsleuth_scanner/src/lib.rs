#![feature(portable_simd)]

use anyhow::{bail, Context, Error, Result};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Pattern {
    pub sig: Vec<u8>,
    pub mask: Vec<u8>,
    pub custom_offset: usize,
    pub captures: Vec<std::ops::Range<usize>>,
    pub xrefs: Vec<(usize, Xref)>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Capture<'data> {
    pub address: usize,
    pub data: &'data [u8],
}

impl TryFrom<String> for Pattern {
    type Error = Error;
    fn try_from(string: String) -> Result<Self, <Self as TryFrom<String>>::Error> {
        Self::new(&string)
    }
}
impl TryFrom<&str> for Pattern {
    type Error = Error;
    fn try_from(string: &str) -> Result<Self, <Self as TryFrom<&str>>::Error> {
        Self::new(string)
    }
}

impl Pattern {
    fn parse_binary_patern(s: &str) -> Option<(u8, u8)> {
        if s.len() == 8 {
            let mut sig = 0;
            let mut mask = 0;
            for (i, b) in s.chars().enumerate() {
                let i = 7 - i;
                match b {
                    '0' => {
                        mask |= 1 << i;
                    }
                    '1' => {
                        sig |= 1 << i;
                        mask |= 1 << i;
                    }
                    '?' => {}
                    _ => return None,
                }
            }
            Some((sig, mask))
        } else {
            None
        }
    }

    fn parse_hex_pattern(s: &str) -> Option<(u8, u8)> {
        if s.len() == 2 {
            let mut sig = 0;
            let mut mask = 0;
            for (i, b) in s.chars().enumerate() {
                let i = (1 - i) * 4;
                if let Some(digit) = b.to_digit(16) {
                    sig |= (digit as u8) << i;
                    mask |= 0xf << i;
                } else if b != '?' {
                    return None;
                }
            }
            Some((sig, mask))
        } else {
            None
        }
    }

    fn parse_maybe_hex(s: &str) -> Result<usize> {
        Ok(s.strip_prefix("0x")
            .map(|s| usize::from_str_radix(s, 16))
            .unwrap_or_else(|| s.parse())?)
    }

    pub fn new(s: &str) -> Result<Self> {
        let mut sig = vec![];
        let mut mask = vec![];
        let mut custom_offset = 0;

        let mut capture_stack = vec![];
        let mut captures = vec![];
        let mut xrefs = vec![];

        let mut i = 0;
        for w in s.split_whitespace() {
            if let Some((s, m)) =
                Self::parse_hex_pattern(w).or_else(|| Self::parse_binary_patern(w))
            {
                if m != 0xff && sig.is_empty() {
                    bail!("first byte cannot be \"??\"");
                } else {
                    sig.push(s);
                    mask.push(m);
                }
                i += 1;
            } else {
                match w {
                    "|" => {
                        custom_offset = i;
                    }
                    "[" => {
                        capture_stack.push(i);
                    }
                    "]" => {
                        if let Some(start) = capture_stack.pop() {
                            captures.push(start..i);
                        } else {
                            bail!("unexpected closing capture at word {i}");
                        }
                    }
                    _ => {
                        if let Some(xref) = w.strip_prefix('X').map(Self::parse_maybe_hex) {
                            let xref =
                                Xref(xref.with_context(|| format!("failed to parse xref {w}"))?);
                            xrefs.push((sig.len(), xref));
                            for _ in 0..4 {
                                sig.push(0);
                                mask.push(0);
                            }
                        } else {
                            bail!("bad pattern word \"{}\"", w)
                        }
                    }
                }
            }
        }
        if let Some(start) = capture_stack.pop() {
            bail!("unclosed capture at word {start}");
        }
        if sig.is_empty() {
            bail!("pattern must match at least one byte");
        }

        Ok(Self {
            sig,
            mask,
            custom_offset,
            captures,
            xrefs,
        })
    }
    /// Create a pattern from a literal `Vec<u8>` with `mask` filled with 0xff and `custom_offset = 0`.
    pub fn from_bytes(sig: Vec<u8>) -> Result<Self> {
        Ok(Self {
            mask: vec![0xff; sig.len()],
            sig,
            custom_offset: 0,
            captures: vec![],
            xrefs: vec![],
        })
    }
    #[inline]
    pub fn is_match(&self, data: &[u8], base_address: usize, index: usize) -> bool {
        for i in 0..self.mask.len() {
            if data[index + i] & self.mask[i] != self.sig[i] {
                return false;
            }
        }
        self.xrefs.iter().all(|(offset, xref)| {
            (base_address + index + offset + 4)
                .checked_add_signed(i32::from_le_bytes(
                    data[index + offset..index + offset + 4].try_into().unwrap(),
                ) as isize)
                .map(|x| x == xref.0)
                .unwrap_or(false)
        })
    }
    pub fn captures<'data>(
        &self,
        data: &'data [u8],
        base_address: usize,
        index: usize,
    ) -> Option<Vec<Capture<'data>>> {
        self.is_match(data, base_address, index).then(|| {
            self.captures
                .iter()
                .map(|c| Capture {
                    address: base_address + index + c.start,
                    data: &data[c.start + index..c.end + index],
                })
                .collect()
        })
    }
    /// compute virtual address from address relative to section as well as account for
    /// custom_offset
    pub fn compute_result(&self, _data: &[u8], base_address: usize, index: usize) -> usize {
        base_address + index + self.custom_offset
    }
}

impl Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut buffer = String::new();
        buffer.push_str(&format!("{:02X}", self.sig[0]));
        for (sig, mask) in self.sig.iter().zip(&self.mask).skip(1) {
            if *mask == 0 {
                buffer.push_str(" ??");
            } else {
                buffer.push_str(&format!(" {:02X}", sig));
            }
        }
        write!(f, "{}", buffer)
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Xref(pub usize);

use std::{
    collections::HashMap,
    fmt::Display,
    simd::{SimdPartialEq, ToBitMask},
};

pub fn scan<'id, ID: Sync>(
    patterns: &[(&'id ID, &Pattern)],
    base_address: usize,
    data: &[u8],
) -> Vec<(&'id ID, usize)> {
    use rayon::prelude::*;

    if patterns.is_empty() {
        return vec![];
    }

    const LANES: usize = 16;

    let max = patterns.iter().map(|(_id, p)| p.sig.len()).max().unwrap();

    let first: Vec<_> = patterns
        .iter()
        .map(|p| (p, std::simd::Simd::splat(p.1.sig[0])))
        .collect();

    // split data for simd
    // cut middle short such that even the longest pattern doesn't have to bounds check
    let (prefix, middle, _suffix) = data[0..data.len().saturating_sub(max)].as_simd::<LANES>();
    let suffix = &data[prefix.len() + middle.len() * LANES..data.len()];

    let mut matches = vec![];

    // prefix
    for (id, p) in patterns {
        for i in 0..prefix.len().min(data.len().saturating_sub(p.sig.len() - 1)) {
            if p.is_match(data, base_address, i) {
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

                            if p.is_match(data, base_address, j) {
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
            if p.is_match(data, base_address, i) {
                matches.push((*id, base_address + i));
            }
        }
    }

    matches
}

pub fn scan_memchr<'id, ID: Sync>(
    patterns: &[(&'id ID, &Pattern)],
    base_address: usize,
    data: &[u8],
) -> Vec<(&'id ID, usize)> {
    use rayon::prelude::*;

    let mut matches = vec![];

    for (id, pattern) in patterns {
        let first_byte_data = &data[0..data.len().saturating_sub(pattern.sig.len() - 1)];
        let chunk_size = (first_byte_data.len()
            / std::thread::available_parallelism()
                .unwrap_or(std::num::NonZeroUsize::new(1).unwrap()))
        .max(1);

        let chunks: Vec<_> = first_byte_data.chunks(chunk_size).enumerate().collect();
        matches.append(
            &mut chunks
                .par_iter()
                .map(|(chunk_index, chunk)| {
                    let mut matches = vec![];
                    let offset = chunk_index * chunk_size;

                    for i in memchr::memchr_iter(pattern.sig[0], chunk) {
                        let j = offset + i;
                        if pattern.is_match(data, base_address, j) {
                            matches.push((*id, pattern.compute_result(data, base_address, j)));
                        }
                    }
                    matches
                })
                .flatten()
                .collect::<Vec<_>>(),
        );
    }
    matches
}

pub fn scan_memchr_lookup<'id, ID: Sync>(
    patterns: &[(&'id ID, &Pattern)],
    base_address: usize,
    data: &[u8],
) -> Vec<(&'id ID, usize)> {
    use rayon::prelude::*;

    if patterns.is_empty() {
        return vec![];
    }

    let mut bins: HashMap<u8, Vec<_>> = Default::default();
    for p in patterns {
        bins.entry(p.1.sig[0]).or_default().push(p);
    }

    let max = patterns.iter().map(|(_id, p)| p.sig.len()).max().unwrap();

    // cut middle short such that even the longest pattern doesn't have to bounds check
    let middle = &data[0..data.len().saturating_sub(max)];

    let mut matches = vec![];

    // middle
    let chunk_size = (middle.len()
        / std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::new(1).unwrap()))
    .max(1);
    let chunks: Vec<_> = middle.chunks(chunk_size).enumerate().collect();
    matches.append(
        &mut chunks
            .par_iter()
            .map(|(index, chunk)| {
                let mut matches = vec![];
                let offset = index * chunk_size;

                for (first, patterns) in &bins {
                    for i in memchr::memchr_iter(*first, chunk) {
                        let j = offset + i;
                        for (id, p) in patterns {
                            if p.is_match(data, base_address, j) {
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
    let start = middle.len();
    for (id, p) in patterns {
        for i in start..start + (data.len() - middle.len()).saturating_sub(p.sig.len() - 1) {
            if p.is_match(data, base_address, i) {
                matches.push((*id, base_address + i));
            }
        }
    }

    matches
}

pub fn scan_xref<'id, ID: Sync>(
    patterns: &[(&'id ID, &Xref)],
    base_address: usize,
    data: &[u8],
) -> Vec<(&'id ID, usize)> {
    use rayon::prelude::*;

    if patterns.is_empty() {
        return vec![];
    }

    let mut matches = vec![];

    let width = 4;

    let first_byte_data = &data[0..data.len().saturating_sub(width - 1)];
    let chunk_size = (first_byte_data.len()
        / std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::new(1).unwrap()))
    .max(1);

    let chunks: Vec<_> = first_byte_data.chunks(chunk_size).enumerate().collect();
    matches.append(
        &mut chunks
            .par_iter()
            .map(|(chunk_index, chunk)| {
                let mut matches = vec![];
                let offset = chunk_index * chunk_size;

                for j in offset..offset + chunk.len() {
                    if let Some(address) = (base_address + width + j).checked_add_signed(
                        i32::from_le_bytes(data[j..j + width].try_into().unwrap())
                            .try_into()
                            .unwrap(),
                    ) {
                        for (id, p) in patterns {
                            if p.0 == address {
                                matches.push((*id, base_address + j));
                            }
                        }
                    }
                }
                matches
            })
            .flatten()
            .collect::<Vec<_>>(),
    );
    matches
}

pub fn scan_xref_binary<'id, ID: Sync>(
    patterns: &[(&'id ID, &Xref)],
    base_address: usize,
    data: &[u8],
) -> Vec<(&'id ID, usize)> {
    use rayon::prelude::*;

    if patterns.is_empty() {
        return vec![];
    }

    let mut patterns = patterns.to_vec();
    patterns.sort_by_key(|p| p.1);

    let mut matches = vec![];

    let width = 4;

    let first_byte_data = &data[0..data.len().saturating_sub(width - 1)];
    let chunk_size = (first_byte_data.len()
        / std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::new(1).unwrap()))
    .max(1);

    let chunks: Vec<_> = first_byte_data.chunks(chunk_size).enumerate().collect();
    matches.append(
        &mut chunks
            .par_iter()
            .map(|(chunk_index, chunk)| {
                let mut matches = vec![];
                let offset = chunk_index * chunk_size;

                for j in offset..offset + chunk.len() {
                    if let Some(address) = (base_address + width + j).checked_add_signed(
                        i32::from_le_bytes(data[j..j + width].try_into().unwrap())
                            .try_into()
                            .unwrap(),
                    ) {
                        if let Ok(i) = patterns.binary_search_by_key(&address, |p| p.1 .0) {
                            // match found
                            let addr = base_address + j;
                            {
                                // walk backwards until unequal
                                let mut i = i - 1;
                                while let Some(prev) = patterns.get(i) {
                                    if prev.1 .0 != address {
                                        break;
                                    }
                                    matches.push((prev.0, addr));
                                    i -= 1;
                                }
                            }
                            {
                                // walk forwards until unequal
                                let mut i = i;
                                while let Some(next) = patterns.get(i) {
                                    if next.1 .0 != address {
                                        break;
                                    }
                                    matches.push((next.0, addr));
                                    i += 1;
                                }
                            }
                        }
                    }
                }
                matches
            })
            .flatten()
            .collect::<Vec<_>>(),
    );
    matches
}

pub fn scan_xref_hash<'id, ID: Sync>(
    patterns: &[(&'id ID, &Xref)],
    base_address: usize,
    data: &[u8],
) -> Vec<(&'id ID, usize)> {
    use rayon::prelude::*;

    if patterns.is_empty() {
        return vec![];
    }

    let patterns = patterns
        .iter()
        .map(|(id, p)| (p.0, *id))
        .collect::<HashMap<_, _>>();

    let mut matches = vec![];

    let width = 4;

    let first_byte_data = &data[0..data.len().saturating_sub(width - 1)];
    let chunk_size = (first_byte_data.len()
        / std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::new(1).unwrap()))
    .max(1);

    let chunks: Vec<_> = first_byte_data.chunks(chunk_size).enumerate().collect();
    matches.append(
        &mut chunks
            .par_iter()
            .map(|(chunk_index, chunk)| {
                let mut matches = vec![];
                let offset = chunk_index * chunk_size;

                for j in offset..offset + chunk.len() {
                    if let Some(address) = (base_address + width + j).checked_add_signed(
                        i32::from_le_bytes(data[j..j + width].try_into().unwrap())
                            .try_into()
                            .unwrap(),
                    ) {
                        if let Some(id) = patterns.get(&address) {
                            matches.push((*id, base_address + j));
                        }
                    }
                }
                matches
            })
            .flatten()
            .collect::<Vec<_>>(),
    );
    matches
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_bits() {
        assert_eq!(None, Pattern::parse_binary_patern("0000000"));
        assert_eq!(None, Pattern::parse_binary_patern("000000000"));
        assert_eq!(Some((0, 0xff)), Pattern::parse_binary_patern("00000000"));
        assert_eq!(
            Some((0b0000_0000, 0b0111_1111)),
            Pattern::parse_binary_patern("?0000000")
        );
        assert_eq!(
            Some((0b0100_0000, 0b0111_1111)),
            Pattern::parse_binary_patern("?1000000")
        );
    }

    #[test]
    fn test_parse_hex() {
        assert_eq!(Some((0xff, 0xff)), Pattern::parse_hex_pattern("ff"));
        assert_eq!(Some((0x00, 0xff)), Pattern::parse_hex_pattern("00"));
        assert_eq!(Some((0x0f, 0x0f)), Pattern::parse_hex_pattern("?f"));
        assert_eq!(Some((0x00, 0x0f)), Pattern::parse_hex_pattern("?0"));
        assert_eq!(Some((0x00, 0xf0)), Pattern::parse_hex_pattern("0?"));
        assert_eq!(None, Pattern::parse_hex_pattern("z0"));
        assert_eq!(None, Pattern::parse_hex_pattern("0"));
        assert_eq!(None, Pattern::parse_hex_pattern("000"));
    }

    #[test]
    fn test_build_pattern() {
        assert!(Pattern::new("?? ??").is_err());
        assert_eq!(
            Pattern {
                sig: vec![0, 0],
                mask: vec![0xff, 0],
                custom_offset: 0,
                captures: vec![],
                xrefs: vec![],
            },
            Pattern::new("00 ??").unwrap()
        );
        assert_eq!(
            Pattern {
                sig: vec![0x10, 0],
                mask: vec![0xff, 0],
                custom_offset: 0,
                captures: vec![],
                xrefs: vec![],
            },
            Pattern::new("10 ??").unwrap()
        );
        assert_eq!(
            Pattern {
                sig: vec![0x10, 0, 0b01010011],
                mask: vec![0xff, 0, 0b11011011],
                custom_offset: 0,
                captures: vec![],
                xrefs: vec![],
            },
            Pattern::new("10 ?? 01?10?11").unwrap()
        );
    }

    #[test]
    fn test_captures() {
        assert!(Pattern::new("?? [ ??").is_err());
        assert!(Pattern::new("?? ] ??").is_err());
        assert!(Pattern::new("[ ] ?? ] ??").is_err());
        assert_eq!(
            Pattern {
                sig: vec![0, 0, 0x10, 0x20],
                mask: vec![0xff, 0, 0xff, 0xff],
                custom_offset: 0,
                captures: vec![2..2, 1..2, 2..4],
                xrefs: vec![],
            },
            Pattern::new("00 [ ?? [ ] ] [ 10 20 ]").unwrap()
        );

        assert_eq!(
            Some(vec![Capture {
                address: 100 + 3,
                data: &[0x99]
            }]),
            Pattern::new("10 20 30 [ ?? ]")
                .unwrap()
                .captures(b"\x10\x20\x30\x99", 100, 0)
        );

        assert_eq!(
            Some(vec![Capture {
                address: 100 + 2,
                data: &[0x30]
            }]),
            Pattern::new("20 [ ?? ]")
                .unwrap()
                .captures(b"\x10\x20\x30\x99\x24", 100, 1)
        );
    }

    type PatternScanFn<'id> = fn(
        patterns: &[(&'id (), &Pattern)],
        base_address: usize,
        data: &[u8],
    ) -> Vec<(&'id (), usize)>;

    type XrefScanFn<'id, ID> = fn(
        patterns: &[(&'id ID, &Xref)],
        base_address: usize,
        data: &[u8],
    ) -> Vec<(&'id ID, usize)>;

    #[test]
    fn test_scan() {
        test_scan_algo(scan);
    }

    #[test]
    fn test_scan_memchr() {
        test_scan_algo(scan_memchr);
    }

    fn test_scan_algo(scan: PatternScanFn) {
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

    #[test]
    fn test_scan_xref() {
        test_scan_xref_algo(scan_xref_binary);
    }

    fn test_scan_xref_algo(scan: XrefScanFn<char>) {
        let scans = [
            (&'a', &Xref(0x504030a)),
            (&'b', &Xref(0x504030a)),
            (&'c', &Xref(0x504030a)),
            (&'d', &Xref(0x504030a)),
        ];

        let mut res = scan(&scans, 3, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        res.sort();
        assert_eq!(
            &[(&'a', 4), (&'b', 4), (&'c', 4), (&'d', 4)],
            res.as_slice()
        );
    }
}
