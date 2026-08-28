use anyhow::{Context, Error, Result, bail};

#[derive(Clone, Eq, PartialEq)]
pub struct PatternSimple {
    pub sig: Vec<u8>,
    pub mask: Vec<u8>,
}
impl PatternSimple {
    #[inline(always)]
    pub fn is_match(&self, data: &[u8], index: usize) -> bool {
        for i in 0..self.len() {
            if data[index + i] & self.mask[i] != self.sig[i] {
                return false;
            }
        }
        true
    }
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.sig.len()
    }
    pub fn iter(&self) -> std::iter::Zip<std::slice::Iter<'_, u8>, std::slice::Iter<'_, u8>> {
        self.sig.iter().zip(&self.mask)
    }
}

fn fmt_byte(f: &mut std::fmt::Formatter<'_>, sig: u8, mask: u8) -> std::fmt::Result {
    if mask == 0 {
        write!(f, "??")?;
    } else if mask == 0xff {
        write!(f, "{sig:02X}")?;
    } else if mask == 0x0f {
        write!(f, "?{:X}", sig & 0xf)?;
    } else if mask == 0xf0 {
        write!(f, "{:X}?", sig >> 4)?;
    } else {
        for bit in (0..8).rev() {
            if mask >> bit & 1 == 1 {
                write!(f, "{}", sig >> bit & 1)?;
            } else {
                write!(f, "?")?;
            }
        }
    }
    Ok(())
}

impl Display for PatternSimple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt_byte(f, self.sig[0], self.mask[0])?;
        for (sig, mask) in self.iter().skip(1) {
            write!(f, " ")?;
            fmt_byte(f, *sig, *mask)?;
        }
        Ok(())
    }
}
impl std::fmt::Debug for PatternSimple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PatternSimple(\"{self}\")")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Pattern {
    pub simple: PatternSimple,
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

    fn parse_maybe_hex_u32(s: &str) -> Result<u32> {
        Ok(s.strip_prefix("0x")
            .map(|s| u32::from_str_radix(s, 16))
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
                        } else if w.starts_with("0x") {
                            sig.extend(u32::to_le_bytes(
                                Self::parse_maybe_hex_u32(w)
                                    .with_context(|| format!("failed to parse 4-bytes hex {w}"))?,
                            ));
                            mask.extend([0xff; 4]);
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
            simple: PatternSimple { sig, mask },
            custom_offset,
            captures,
            xrefs,
        })
    }
    /// Create a pattern from a literal `Vec<u8>` with `mask` filled with 0xff and `custom_offset = 0`.
    pub fn from_bytes(sig: Vec<u8>) -> Result<Self> {
        Ok(Self {
            simple: PatternSimple {
                mask: vec![0xff; sig.len()],
                sig,
            },
            custom_offset: 0,
            captures: vec![],
            xrefs: vec![],
        })
    }
    #[inline(always)]
    pub fn is_match(&self, data: &[u8], base_address: usize, index: usize) -> bool {
        self.simple.is_match(data, index)
            && self.xrefs.iter().all(|(offset, xref)| {
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
        fmt_byte(f, self.simple.sig[0], self.simple.mask[0])?;
        let mut iter = self.simple.iter().enumerate().skip(1);
        while let Some((i, (sig, mask))) = iter.next() {
            if i == self.custom_offset {
                write!(f, " |")?;
            }
            write!(f, " ")?;
            if *mask == 0
                && let Some((_offset, xref)) =
                    self.xrefs.iter().find(|(offset, _xref)| *offset == i)
            {
                write!(f, "X0x{:X}", xref.0)?;
                iter.nth(2); // skip 3
                continue;
            }
            fmt_byte(f, *sig, *mask)?;
        }
        Ok(())
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
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Display,
};

#[derive(Debug, Eq, PartialEq)]
struct PatternPair<'p> {
    pattern: &'p Pattern,
    partial: PatternSimple,
    offset: usize,
}
impl PatternPair<'_> {
    #[inline(always)]
    fn add_match(
        &self,
        data: &[u8],
        base_address: usize,
        offset: usize,
        pattern_index: usize,
        matches: &mut Vec<(usize, usize)>,
    ) {
        if self.partial.is_match(data, offset)
            && offset >= self.offset
            && self
                .pattern
                .is_match(data, base_address, offset - self.offset)
        {
            matches.push((
                pattern_index,
                self.pattern
                    .compute_result(data, base_address, offset - self.offset),
            ));
        }
    }
}

/// Estimated occurrence count of each byte value in the data being scanned.
///
/// Only used to rank candidate anchor bytes against each other, so a sampled
/// estimate is plenty -- see [`sample_histogram`].
type Histogram = [u64; 256];

/// Estimate how often each byte value occurs in `data`.
///
/// Sampling a block out of every `SAMPLE_RATE` keeps this ~free relative to the
/// scan it feeds, while staying spatially spread out so that byte values which
/// only cluster in one region of a section are still accounted for.
fn sample_histogram(data: &[u8]) -> Histogram {
    use rayon::prelude::*;

    const BLOCK: usize = 4096;
    const SAMPLE_RATE: usize = 32;
    const STRIDE: usize = BLOCK * SAMPLE_RATE;

    fn count(data: &[u8], hist: &mut Histogram) {
        for b in data {
            hist[*b as usize] += 1;
        }
    }

    if data.len() <= STRIDE {
        let mut hist = [0; 256];
        count(data, &mut hist);
        return hist;
    }

    let (mut hist, sampled) = data
        .par_chunks(4 << 20)
        .map(|chunk| {
            let mut hist = [0; 256];
            let mut sampled = 0;
            for block in chunk.chunks(STRIDE) {
                let block = &block[..BLOCK.min(block.len())];
                count(block, &mut hist);
                sampled += block.len() as u64;
            }
            (hist, sampled)
        })
        .reduce(
            || ([0; 256], 0),
            |mut a: (Histogram, u64), b: (Histogram, u64)| {
                for (a, b) in a.0.iter_mut().zip(b.0) {
                    *a += b;
                }
                (a.0, a.1 + b.1)
            },
        );

    // scale the sample back up so counts are comparable to `data.len()`
    let scale = data.len() as u64 / sampled.max(1);
    for count in &mut hist {
        *count *= scale;
    }
    hist
}

/// Estimated number of candidates at `pos` that will survive the cheap prefix
/// check the scan does before verifying a pattern in full.
///
/// [`scan_pattern`] bins patterns by the first 4 fixed bytes of their partial,
/// else the first 2, else just the first, so an anchor followed by more fixed
/// bytes rejects far more candidates before any real work happens. Byte
/// probabilities are treated as independent, which is crude but enough to tell
/// "anchor mid-pattern with only wildcards after it" apart from "anchor on a
/// fixed run".
fn surviving_candidates(hist: &Histogram, data_len: u64, p: &PatternSimple, pos: usize) -> u64 {
    let fixed_run = (pos..p.len()).take_while(|i| p.mask[*i] == 0xff).count();
    let window = match fixed_run {
        0..=1 => 1,
        2..=3 => 2,
        _ => 4,
    };
    let mut est = hist[p.sig[pos] as usize];
    for i in 1..window {
        est = est.saturating_mul(hist[p.sig[pos + i] as usize]) / data_len.max(1);
    }
    est
}

/// The patterns sharing one bin key, plus whatever filter they have in common.
#[derive(Debug)]
struct Bin<'p> {
    patterns: Vec<(usize, &'p PatternPair<'p>)>,
    xref: Option<SharedXref>,
}

/// A single xref that every pattern in a bin has, at the same place relative to
/// the anchor byte.
///
/// Resolving it once per candidate and rejecting the whole bin on a miss is what
/// keeps "some instruction referencing one of these addresses" cheap to scan --
/// those turn into many patterns that differ only in encoding, and checking the
/// reference per pattern repeats the same work for each.
#[derive(Debug)]
struct SharedXref {
    /// where the rel32 sits relative to the anchor byte
    offset: usize,
    /// sorted and deduplicated, so a miss costs one binary search
    targets: Vec<usize>,
}

impl<'p> Bin<'p> {
    fn new(patterns: Vec<(usize, &'p PatternPair<'p>)>) -> Self {
        Self {
            xref: SharedXref::new(&patterns),
            patterns,
        }
    }
    #[inline(always)]
    fn add_matches(
        &self,
        data: &[u8],
        base_address: usize,
        offset: usize,
        matches: &mut Vec<(usize, usize)>,
    ) {
        if let Some(xref) = &self.xref
            && !xref.is_match(data, base_address, offset)
        {
            return;
        }
        for (pattern_index, pair) in &self.patterns {
            pair.add_match(data, base_address, offset, *pattern_index, matches)
        }
    }
}

impl SharedXref {
    fn new(patterns: &[(usize, &PatternPair<'_>)]) -> Option<Self> {
        let mut offset = None;
        let mut targets = Vec::with_capacity(patterns.len());
        for (_, pair) in patterns {
            let [(xref_offset, xref)] = pair.pattern.xrefs[..] else {
                return None;
            };
            // relative to the anchor byte rather than the start of the pattern,
            // so that patterns anchored at different positions can still share
            let relative = xref_offset.checked_sub(pair.offset)?;
            if *offset.get_or_insert(relative) != relative {
                return None;
            }
            targets.push(xref.0);
        }
        targets.sort_unstable();
        targets.dedup();
        Some(Self {
            offset: offset?,
            targets,
        })
    }

    /// Whether the rel32 near the candidate anchor at `offset` points at any
    /// target.
    ///
    /// Reads within the longest pattern of the scan, which the caller has
    /// already trimmed the data to accommodate.
    #[inline(always)]
    fn is_match(&self, data: &[u8], base_address: usize, offset: usize) -> bool {
        let at = offset + self.offset;
        let relative = i32::from_le_bytes(data[at..at + 4].try_into().unwrap());
        (base_address + at + 4)
            .checked_add_signed(relative as isize)
            .is_some_and(|target| self.targets.binary_search(&target).is_ok())
    }
}

/// Assign every pattern an anchor byte to be located with `memchr`.
///
/// Anchoring is what determines how much work the scan does, and it is not the
/// `memchr` pass that costs -- it is the candidates that pass produces. Each
/// distinct anchor pays for its pass and for a bin lookup per candidate, and
/// each pattern then pays for the candidates that reach its bin. So anchors are
/// picked greedily by cost per pattern anchored, judging a byte on how rare it
/// is in the data actually being scanned and on how much fixed context follows
/// it.
fn group_patterns<'p>(
    patterns: &[&'p Pattern],
    hist: &Histogram,
    data_len: usize,
) -> Vec<PatternPair<'p>> {
    let mut pattern_pairs: Vec<Option<PatternPair>> = patterns.iter().map(|_| None).collect();

    // Cost of one more `memchr` pass, in candidates. Anchor bytes rarer than
    // this are effectively free, so sharing is only worth it when the shared
    // byte isn't much more common than the rare ones it replaces.
    let pass_cost = (data_len / 1024) as u64;

    let mut remaining: Vec<Option<&Pattern>> = patterns.iter().map(|p| Some(*p)).collect();
    while remaining.iter().any(Option::is_some) {
        // every fixed byte, and where it first appears in each of the
        // still-unassigned patterns it could anchor
        let mut candidates: BTreeMap<u8, Vec<(u64, usize, usize)>> = Default::default();
        for (pi, p) in remaining
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.map(|p| (i, &p.simple)))
        {
            let mut seen = [false; 256];
            for (pos, (sig, mask)) in p.iter().enumerate() {
                if *mask == 0xff && !std::mem::replace(&mut seen[*sig as usize], true) {
                    let est = surviving_candidates(hist, data_len as u64, p, pos);
                    candidates.entry(*sig).or_default().push((est, pi, pos));
                }
            }
        }

        // `a / a_len` against `b / b_len`, cross-multiplied to stay in integers
        let cheaper = |(a, a_len): (u128, usize), (b, b_len): (u128, usize)| {
            (a * b_len as u128).cmp(&(b * a_len as u128))
        };

        // For each byte, how many patterns are worth anchoring on it. A pattern
        // with little fixed context after the anchor is expensive wherever it
        // goes, and letting it drag the average up would rule out a byte that is
        // a good anchor for everything else -- so consider the patterns cheapest
        // first and stop where the cost per pattern stops improving.
        let best = candidates
            .iter_mut()
            .map(|(byte, anchored)| {
                anchored.sort_unstable();
                let mut total = (pass_cost + hist[*byte as usize]) as u128;
                let (mut best, mut best_len) = (u128::MAX, 1);
                for (len, (est, _, _)) in anchored.iter().enumerate() {
                    total += *est as u128;
                    if cheaper((total, len + 1), (best, best_len)).is_lt() {
                        (best, best_len) = (total, len + 1);
                    }
                }
                (best, best_len, *byte, &anchored[..best_len])
            })
            .min_by(|a, b| cheaper((a.0, a.1), (b.0, b.1)).then(b.1.cmp(&a.1)));

        let Some((_, _, _, anchored)) = best else {
            // every fixed byte belongs to an already-assigned pattern, so what's
            // left has none at all and there is nothing to anchor on
            panic!(
                "pattern has no fixed byte to anchor on: {:?}",
                remaining.iter().flatten().next().unwrap()
            );
        };

        for (_, pi, pos) in anchored {
            let p = remaining[*pi].take().unwrap();
            pattern_pairs[*pi] = Some(PatternPair {
                pattern: p,
                partial: PatternSimple {
                    sig: p.simple.sig[*pos..].to_vec(),
                    mask: p.simple.mask[*pos..].to_vec(),
                },
                offset: *pos,
            });
        }
    }
    pattern_pairs.into_iter().flatten().collect()
}

pub fn scan_pattern(patterns: &[&Pattern], base_address: usize, data: &[u8]) -> Vec<Vec<usize>> {
    use rayon::prelude::*;

    let mut result_bins = patterns.iter().map(|_| vec![]).collect::<Vec<_>>();

    if patterns.is_empty() {
        return result_bins;
    }

    const WIDE1: usize = 2;
    const WIDE2: usize = 4;

    let pattern_pairs = group_patterns(patterns, &sample_histogram(data), data.len());

    let mut all_bins = HashSet::new();
    let mut short_bins: HashMap<u8, Vec<_>> = Default::default();
    let mut wide1_bins: HashMap<[u8; WIDE1], Vec<_>> = Default::default();
    let mut wide2_bins: HashMap<[u8; WIDE2], Vec<_>> = Default::default();
    for (pi, pair) in pattern_pairs.iter().enumerate() {
        let p = &pair.partial;

        all_bins.insert(p.sig[0]);
        if p.mask.iter().take(WIDE2).filter(|m| **m == 0xff).count() == WIDE2 {
            let mut buf = [0; WIDE2];
            buf.copy_from_slice(&p.sig[0..WIDE2]);
            wide2_bins.entry(buf).or_default().push((pi, pair));
        } else if p.mask.iter().take(WIDE1).filter(|m| **m == 0xff).count() == WIDE1 {
            let mut buf = [0; WIDE1];
            buf.copy_from_slice(&p.sig[0..WIDE1]);
            wide1_bins.entry(buf).or_default().push((pi, pair));
        } else {
            short_bins.entry(p.sig[0]).or_default().push((pi, pair));
        }
    }
    let all_bins = Vec::from_iter(all_bins);

    fn into_bins<'p, K: std::hash::Hash + Eq>(
        bins: HashMap<K, Vec<(usize, &'p PatternPair<'p>)>>,
    ) -> HashMap<K, Bin<'p>> {
        bins.into_iter().map(|(k, v)| (k, Bin::new(v))).collect()
    }
    let short_bins = into_bins(short_bins);
    let wide1_bins = into_bins(wide1_bins);
    let wide2_bins = into_bins(wide2_bins);

    let max = patterns.iter().map(|p| p.simple.len()).max().unwrap();

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
                        if let Some(bin) = short_bins.get(first) {
                            bin.add_matches(data, base_address, j, &mut matches)
                        }
                        if !wide2_bins.is_empty() {
                            let mut buf = [0; WIDE2];
                            buf.copy_from_slice(&data[j..j + WIDE2]);
                            if let Some(bin) = wide2_bins.get(&buf) {
                                bin.add_matches(data, base_address, j, &mut matches)
                            }
                        }
                        if !wide1_bins.is_empty() {
                            let mut buf = [0; WIDE1];
                            buf.copy_from_slice(&data[j..j + WIDE1]);
                            if let Some(bin) = wide1_bins.get(&buf) {
                                bin.add_matches(data, base_address, j, &mut matches)
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
    for (pi, p) in pattern_pairs.iter().enumerate() {
        for i in (start.saturating_sub(p.offset))
            ..start + (data.len() - middle.len()).saturating_sub(p.pattern.simple.len() - 1)
        {
            if p.pattern.is_match(data, base_address, i) {
                matches.push((pi, base_address + i));
            }
        }
    }

    for (pi, addr) in matches {
        result_bins[pi].push(addr);
    }

    result_bins
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
                    ) && let Ok(i) = patterns.binary_search_by_key(&address, |p| p.0)
                    {
                        // match found
                        let addr = base_address + j;
                        {
                            // walk backwards until unequal
                            let mut i = i;
                            while let Some(prev) = (i > 0).then(|| patterns.get(i - 1)).flatten() {
                                if prev.0 != address {
                                    break;
                                }
                                matches.push((i - 1, addr));
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
                simple: PatternSimple {
                    sig: vec![0, 0],
                    mask: vec![0xff, 0],
                },
                custom_offset: 0,
                captures: vec![],
                xrefs: vec![],
            },
            Pattern::new("00 ??").unwrap()
        );
        assert_eq!(
            Pattern {
                simple: PatternSimple {
                    sig: vec![0x10, 0],
                    mask: vec![0xff, 0],
                },
                custom_offset: 0,
                captures: vec![],
                xrefs: vec![],
            },
            Pattern::new("10 ??").unwrap()
        );
        assert_eq!(
            Pattern {
                simple: PatternSimple {
                    sig: vec![0x10, 0, 0b01010011],
                    mask: vec![0xff, 0, 0b11011011],
                },
                custom_offset: 0,
                captures: vec![],
                xrefs: vec![],
            },
            Pattern::new("10 ?? 01?10?11").unwrap()
        );
    }

    #[test]
    fn test_display_pattern() {
        assert_eq!(
            Pattern::new("12 34 | 56").unwrap().to_string(),
            "12 34 | 56"
        );
        assert_eq!(
            Pattern::new("12 34 | 56").unwrap().simple.to_string(),
            "12 34 56"
        );

        assert_eq!(Pattern::new("12 34 56").unwrap().to_string(), "12 34 56");
        assert_eq!(
            Pattern::new("12 34 56").unwrap().simple.to_string(),
            "12 34 56"
        );

        assert_eq!(
            Pattern::new("12 X0x34 56").unwrap().to_string(),
            "12 X0x34 56"
        );
        assert_eq!(
            Pattern::new("12 X0x34 56").unwrap().simple.to_string(),
            "12 ?? ?? ?? ?? 56"
        );

        assert_eq!(
            Pattern::new("12 ?3 45").unwrap().simple.to_string(),
            "12 ?3 45"
        );
        assert_eq!(
            Pattern::new("12 3? 45").unwrap().simple.to_string(),
            "12 3? 45"
        );
        assert_eq!(
            Pattern::new("12 ?0000001 45").unwrap().simple.to_string(),
            "12 ?0000001 45"
        );
        assert_eq!(
            Pattern::new("12 ??100??1 45").unwrap().simple.to_string(),
            "12 ??100??1 45"
        );

        // a masked leading byte, as the collapsed instruction-encoding patterns use
        assert_eq!(
            Pattern::new("01001?00 8D 00???101 X0x1234")
                .unwrap()
                .to_string(),
            "01001?00 8D 00???101 X0x1234"
        );
        assert_eq!(Pattern::new("?? 12").unwrap().to_string(), "?? 12");
    }

    #[test]
    fn test_captures() {
        assert!(Pattern::new("?? [ ??").is_err());
        assert!(Pattern::new("?? ] ??").is_err());
        assert!(Pattern::new("[ ] ?? ] ??").is_err());
        assert_eq!(
            Pattern {
                simple: PatternSimple {
                    sig: vec![0, 0, 0x10, 0x20],
                    mask: vec![0xff, 0, 0xff, 0xff],
                },
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

    /// 1 MiB of data, so one extra `memchr` pass is priced at 1024 candidates
    const DATA_LEN: usize = 1 << 20;

    /// Every byte equally common except the ones listed, so anchor choice comes
    /// down to whatever the caller is actually testing
    fn hist_with(counts: &[(u8, u64)]) -> Histogram {
        let mut hist = [1000; 256];
        for (byte, count) in counts {
            hist[*byte as usize] = *count;
        }
        hist
    }

    #[test]
    fn test_group_patterns() {
        // simple
        assert_eq!(
            group_patterns(
                &[
                    &Pattern::new("12 34").unwrap(),
                    &Pattern::new("34 56").unwrap(),
                ],
                &hist_with(&[(0x34, 10)]),
                DATA_LEN
            ),
            vec![
                PatternPair {
                    pattern: &Pattern::new("12 34").unwrap(),
                    partial: Pattern::new("34").unwrap().simple,
                    offset: 1,
                },
                PatternPair {
                    pattern: &Pattern::new("34 56").unwrap(),
                    partial: Pattern::new("34 56").unwrap().simple,
                    offset: 0,
                },
            ]
        );

        // duplicate bytes
        assert_eq!(
            group_patterns(
                &[
                    &Pattern::new("12 12 12 34").unwrap(),
                    &Pattern::new("34 56 12").unwrap(),
                ],
                &hist_with(&[(0x12, 10)]),
                DATA_LEN
            ),
            vec![
                PatternPair {
                    pattern: &Pattern::new("12 12 12 34").unwrap(),
                    partial: Pattern::new("12 12 12 34").unwrap().simple,
                    offset: 0,
                },
                PatternPair {
                    pattern: &Pattern::new("34 56 12").unwrap(),
                    partial: Pattern::new("12").unwrap().simple,
                    offset: 2,
                },
            ]
        );

        // multiple possible groupings (use first)
        assert_eq!(
            group_patterns(
                &[
                    &Pattern::new("12 34 56").unwrap(),
                    &Pattern::new("34 56").unwrap(),
                ],
                &hist_with(&[(0x34, 10)]),
                DATA_LEN
            ),
            vec![
                PatternPair {
                    pattern: &Pattern::new("12 34 56").unwrap(),
                    partial: Pattern::new("34 56").unwrap().simple,
                    offset: 1,
                },
                PatternPair {
                    pattern: &Pattern::new("34 56").unwrap(),
                    partial: Pattern::new("34 56").unwrap().simple,
                    offset: 0,
                },
            ]
        );

        // a byte common in the data is avoided even though it is the only one
        // both patterns share (this used to be a hardcoded ban list)
        assert_eq!(
            group_patterns(
                &[
                    &Pattern::new("12 00").unwrap(),
                    &Pattern::new("00 56").unwrap(),
                ],
                &hist_with(&[(0x00, 1_000_000)]),
                DATA_LEN
            ),
            vec![
                PatternPair {
                    pattern: &Pattern::new("12 00").unwrap(),
                    partial: Pattern::new("12 00").unwrap().simple,
                    offset: 0,
                },
                PatternPair {
                    pattern: &Pattern::new("00 56").unwrap(),
                    partial: Pattern::new("56").unwrap().simple,
                    offset: 1,
                },
            ]
        );

        // a common byte is still used when it is the only byte available
        assert_eq!(
            group_patterns(
                &[&Pattern::new("00").unwrap()],
                &hist_with(&[(0x00, 1_000_000)]),
                DATA_LEN
            ),
            vec![PatternPair {
                pattern: &Pattern::new("00").unwrap(),
                partial: Pattern::new("00").unwrap().simple,
                offset: 0,
            },]
        );
    }

    #[test]
    fn test_group_patterns_prefers_rare_bytes() {
        let patterns = [
            &Pattern::new("AA 11").unwrap(),
            &Pattern::new("AA 22").unwrap(),
            &Pattern::new("AA 33").unwrap(),
        ];

        // 0xAA is the only byte all three share, but it is common enough that
        // three passes on rare bytes come out cheaper than one pass on it
        let anchors = group_patterns(
            &patterns,
            &hist_with(&[(0xAA, 1_000_000), (0x11, 10), (0x22, 10), (0x33, 10)]),
            DATA_LEN,
        );
        assert_eq!(
            anchors.iter().map(|p| p.offset).collect::<Vec<_>>(),
            vec![1, 1, 1]
        );

        // when 0xAA is only marginally more common, sharing one pass wins
        let anchors = group_patterns(&patterns, &hist_with(&[(0xAA, 2000)]), DATA_LEN);
        assert_eq!(
            anchors.iter().map(|p| p.offset).collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
    }

    #[test]
    fn test_sample_histogram() {
        // small enough to be counted exactly
        let hist = sample_histogram(&[1, 1, 1, 2]);
        assert_eq!((hist[1], hist[2], hist[3]), (3, 1, 0));

        // large enough to be sampled, so counts are estimates: 3/4 of the data
        // is 0x01 and 1/4 is 0x02, spread evenly
        let data: Vec<u8> = std::iter::repeat_n([1, 1, 1, 2], 4 << 20)
            .flatten()
            .collect();
        let hist = sample_histogram(&data);
        let total = data.len() as u64;
        assert!(hist[1].abs_diff(total / 4 * 3) < total / 100, "{}", hist[1]);
        assert!(hist[2].abs_diff(total / 4) < total / 100, "{}", hist[2]);
        assert_eq!(hist[3], 0);
    }

    #[test]
    fn test_scan_shared_xref() {
        // three `lea r64, [rip+disp32]`, differing in register and REX, the
        // first two pointing at the same address. Patterns like these share a
        // bin and are filtered on the referenced address rather than one at a
        // time, so make sure that filter agrees with matching them individually.
        let base = 0x1000;
        let mut data = vec![
            0x48, 0x8d, 0x0d, 0x05, 0x00, 0x00, 0x00, // lea rcx, [rip+5]  -> 0x100c
            0x4c, 0x8d, 0x05, 0xfe, 0xff, 0xff, 0xff, // lea r8,  [rip-2]  -> 0x100c
            0x48, 0x8d, 0x15, 0x00, 0x00, 0x00, 0x00, // lea rdx, [rip+0]  -> 0x1015
        ];
        data.resize(data.len() + 7, 0); // so every instruction is off the suffix

        let lea = |target| Pattern::new(format!("01001?00 8d 00???101 X0x{target:X}")).unwrap();
        let (hit, other, miss) = (lea(0x100c), lea(0x1015), lea(0x100d));

        assert_eq!(
            scan_pattern(&[&hit, &other, &miss], base, &data),
            vec![vec![0x1000, 0x1007], vec![0x100e], vec![]]
        );

        // and each on its own, where there is nothing to share the filter with
        assert_eq!(
            scan_pattern(&[&hit], base, &data),
            vec![vec![0x1000, 0x1007]]
        );
        assert_eq!(
            scan_pattern(&[&miss], base, &data),
            vec![Vec::<usize>::new()]
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
        let base = 123;

        let data = vec![1; len + lanes];
        let matches: Vec<_> = (base..len + base).collect();

        for i in 0..lanes {
            let slice = &data[i..i + len];
            assert_eq!(vec![matches.clone()], scan(&patterns, base, slice));
        }

        let patterns = [&Pattern::new("01 02").unwrap()];

        // obtuse generator to test every combination of chunk boundaries
        let data: Vec<_> = std::iter::repeat_n([1, 2, 3], 32).flatten().collect();
        let matches: Vec<_> = (0..3)
            .map(|offset| {
                (0..len / 3)
                    .map(|i| i * 3 + offset + base)
                    .collect::<Vec<_>>()
            })
            .collect();

        for i in 0..(len - lanes) {
            let slice = &data[i..i + len];
            let res = scan(&patterns, base, slice);
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
