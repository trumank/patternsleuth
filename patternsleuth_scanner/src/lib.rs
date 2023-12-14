use anyhow::{bail, Context, Error, Result};

#[derive(Clone, Eq, PartialEq)]
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
        Self::new(string)
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

    pub fn new<S: AsRef<str>>(s: S) -> Result<Self> {
        let mut sig = vec![];
        let mut mask = vec![];
        let mut custom_offset = 0;

        let mut capture_stack = vec![];
        let mut captures = vec![];
        let mut xrefs = vec![];

        let mut i = 0;
        for w in s.as_ref().split_whitespace() {
            if let Some((s, m)) =
                Self::parse_hex_pattern(w).or_else(|| Self::parse_binary_patern(w))
            {
                sig.push(s);
                mask.push(m);
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
                            i += 4;
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
            // TODO xrefs and bit masks
            if *mask == 0 {
                buffer.push_str(" ??");
            } else {
                buffer.push_str(&format!(" {:02X}", sig));
            }
        }
        write!(f, "{}", buffer)
    }
}
impl std::fmt::Debug for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pattern(\"{self}\")")
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Xref(pub usize);

use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
};

pub fn scan_pattern(patterns: &[&Pattern], base_address: usize, data: &[u8]) -> Vec<Vec<usize>> {
    use rayon::prelude::*;

    let mut bins = patterns.iter().map(|_| vec![]).collect::<Vec<_>>();

    if patterns.is_empty() {
        return bins;
    }

    const WIDE: usize = 4;

    let mut all_bins = HashSet::new();
    let mut short_bins: HashMap<u8, Vec<_>> = Default::default();
    let mut wide_bins: HashMap<[u8; WIDE], Vec<_>> = Default::default();
    for (pi, p) in patterns.iter().enumerate() {
        all_bins.insert(p.sig[0]);
        if p.mask.iter().take(WIDE).filter(|m| **m == 0xff).count() == WIDE {
            let mut buf = [0; WIDE];
            buf.copy_from_slice(&p.sig[0..WIDE]);
            wide_bins.entry(buf).or_default().push((pi, p));
        } else {
            short_bins.entry(p.sig[0]).or_default().push((pi, p));
        }
    }
    let all_bins = Vec::from_iter(all_bins);

    let max = patterns.iter().map(|p| p.sig.len()).max().unwrap();

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

                for first in &all_bins {
                    for i in memchr::memchr_iter(*first, chunk) {
                        let j = offset + i;
                        if let Some(patterns) = short_bins.get(first) {
                            for (pi, p) in patterns.iter() {
                                if p.is_match(data, base_address, j) {
                                    matches.push((*pi, p.compute_result(data, base_address, j)));
                                }
                            }
                        }
                        if !wide_bins.is_empty() {
                            let mut buf = [0; WIDE];
                            buf.copy_from_slice(&data[j..j + WIDE]);
                            if let Some(patterns) = wide_bins.get(&buf) {
                                for (pi, p) in patterns.iter() {
                                    if p.is_match(data, base_address, j) {
                                        matches
                                            .push((*pi, p.compute_result(data, base_address, j)));
                                    }
                                }
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
    for (pi, p) in patterns.iter().enumerate() {
        for i in start..start + (data.len() - middle.len()).saturating_sub(p.sig.len() - 1) {
            if p.is_match(data, base_address, i) {
                matches.push((pi, base_address + i));
            }
        }
    }

    for (pi, addr) in matches {
        bins[pi].push(addr);
    }

    bins
}

pub fn scan_xref(patterns: &[&Xref], base_address: usize, data: &[u8]) -> Vec<Vec<usize>> {
    use rayon::prelude::*;

    let mut bins = patterns.iter().map(|_| vec![]).collect::<Vec<_>>();

    if patterns.is_empty() {
        return bins;
    }

    let mut patterns = patterns.to_vec();
    patterns.sort();

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
                        if let Ok(i) = patterns.binary_search_by_key(&address, |p| p.0) {
                            // match found
                            let addr = base_address + j;
                            {
                                // walk backwards until unequal
                                let mut i = i - 1;
                                while let Some(prev) = patterns.get(i) {
                                    if prev.0 != address {
                                        break;
                                    }
                                    matches.push((i, addr));
                                    i -= 1;
                                }
                            }
                            {
                                // walk forwards until unequal
                                let mut i = i;
                                while let Some(next) = patterns.get(i) {
                                    if next.0 != address {
                                        break;
                                    }
                                    matches.push((i, addr));
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

    for (pi, addr) in matches {
        bins[pi].push(addr);
    }

    bins
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
        assert!(Pattern::new("?? ??").is_ok());
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

    type PatternScanFn =
        fn(patterns: &[&Pattern], base_address: usize, data: &[u8]) -> Vec<Vec<usize>>;

    type XrefScanFn = fn(patterns: &[&Xref], base_address: usize, data: &[u8]) -> Vec<Vec<usize>>;

    #[test]
    fn test_scan_pattern() {
        test_scan_algo(scan_pattern);
    }

    fn test_scan_algo(scan: PatternScanFn) {
        let patterns = [&Pattern::new("01").unwrap()];

        let len = 64;
        let lanes = 32;

        let data = vec![1; len + lanes];
        let matches: Vec<_> = (0..len).collect();

        for i in 0..lanes {
            let slice = &data[i..i + len];
            assert_eq!(vec![matches.clone()], scan(&patterns, 0, slice));
        }

        let patterns = [&Pattern::new("01 02").unwrap()];

        // obtuse generator to test every combination of chunk boundaries
        let data: Vec<_> = std::iter::repeat([1, 2, 3]).take(32).flatten().collect();
        let matches: Vec<_> = (0..3)
            .map(|offset| (0..len / 3).map(|i| i * 3 + offset).collect::<Vec<_>>())
            .collect();

        for i in 0..(len - lanes) {
            let slice = &data[i..i + len];
            let res = scan(&patterns, 0, slice);
            assert_eq!(vec![matches[(3 - (i % 3)) % 3].clone()], res);
        }
    }

    #[test]
    fn test_scan_xref() {
        test_scan_xref_algo(scan_xref);
    }

    fn test_scan_xref_algo(scan: XrefScanFn) {
        let scans = [
            &Xref(0x504030a),
            &Xref(0x504030a),
            &Xref(0x504030a),
            &Xref(0x504030a),
        ];

        let mut res = scan(&scans, 3, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        res.sort();
        assert_eq!(vec![vec![4], vec![4], vec![4], vec![4]], res);
    }
}
