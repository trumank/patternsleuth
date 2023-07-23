use std::{
    collections::HashMap,
    simd::{SimdPartialEq, ToBitMask},
};

use super::Pattern;

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
            if p.is_match(data, i) {
                matches.push((*id, base_address + i));
            }
        }
    }

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

                for (i, chunk) in chunk.iter().enumerate() {
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

pub fn scan_lookup<'id, ID: Sync>(
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

    let mut bins: HashMap<u8, Vec<_>> = Default::default();
    for p in patterns {
        bins.entry(p.1.sig[0]).or_default().push(p);
    }

    let first: Vec<_> = bins
        .into_iter()
        .map(|(first, patterns)| (std::simd::Simd::splat(first), patterns))
        .collect();

    // split data for simd
    // cut middle short such that even the longest pattern doesn't have to bounds check
    let (prefix, middle, _suffix) = data[0..data.len().saturating_sub(max)].as_simd::<LANES>();
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

                for (i, chunk) in chunk.iter().enumerate() {
                    for (f, patterns) in &first {
                        let mut mask = f.simd_eq(*chunk).to_bitmask();

                        while mask != 0 {
                            let next = mask.trailing_zeros();
                            mask ^= 1 << next;

                            let j = prefix.len() + (offset + i) * LANES + next as usize;

                            for (id, p) in patterns {
                                if p.is_match(data, j) {
                                    matches.push((*id, p.compute_result(data, base_address, j)));
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
                        if pattern.is_match(data, j) {
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

pub fn scan_memchr_lookup<'id, ID: Sync + std::fmt::Debug>(
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
    let start = middle.len();
    for (id, p) in patterns {
        for i in start..start + (data.len() - middle.len()).saturating_sub(p.sig.len() - 1) {
            if p.is_match(data, i) {
                matches.push((*id, base_address + i));
            }
        }
    }

    matches
}

pub fn scan_simple<'id, ID: Sync>(
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

                    for i in 0..chunk.len() {
                        let j = offset + i;
                        if pattern.is_match(data, j) {
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

pub fn scan_simple_batched<'id, ID: Sync>(
    patterns: &[(&'id ID, &Pattern)],
    base_address: usize,
    data: &[u8],
) -> Vec<(&'id ID, usize)> {
    use rayon::prelude::*;

    if patterns.is_empty() {
        return vec![];
    }

    let max = patterns.iter().map(|(_id, p)| p.sig.len()).max().unwrap();
    let first: Vec<_> = patterns.iter().map(|p| p.1.sig[0]).collect();

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

                for (i, _) in chunk.iter().enumerate() {
                    for (pattern_index, first_byte) in first.iter().enumerate() {
                        let j = offset + i;

                        if data[j] == *first_byte {
                            let (id, p) = &patterns[pattern_index];
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
    let start = middle.len();
    for (id, p) in patterns {
        for i in start..start + (data.len() - middle.len()).saturating_sub(p.sig.len() - 1) {
            if p.is_match(data, i) {
                matches.push((*id, base_address + i));
            }
        }
    }

    matches
}

pub fn scan_simple_batched_lookup<'id, ID: Sync + std::fmt::Debug>(
    patterns: &[(&'id ID, &Pattern)],
    base_address: usize,
    data: &[u8],
) -> Vec<(&'id ID, usize)> {
    use rayon::prelude::*;

    if patterns.is_empty() {
        return vec![];
    }

    let max = patterns.iter().map(|(_id, p)| p.sig.len()).max().unwrap();

    let mut bins: Vec<Vec<(&ID, &Pattern)>> = (0..0x100).map(|_| vec![]).collect();
    for pattern in patterns {
        bins[pattern.1.sig[0] as usize].push(*pattern);
    }

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

                for (i, _) in chunk.iter().enumerate() {
                    let j = offset + i;
                    for (id, p) in &bins[data[j] as usize] {
                        if p.is_match(data, j) {
                            matches.push((*id, p.compute_result(data, base_address, j)));
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

    type ScanFn<'id> = fn(
        patterns: &[(&'id (), &Pattern)],
        base_address: usize,
        data: &[u8],
    ) -> Vec<(&'id (), usize)>;

    #[test]
    fn test_scan() {
        test_scan_algo(scan);
    }

    #[test]
    fn test_scan_lookup() {
        test_scan_algo(scan_lookup);
    }

    #[test]
    fn test_scan_memchr() {
        test_scan_algo(scan_memchr);
    }

    #[test]
    fn test_scan_memchr_lookup() {
        test_scan_algo(scan_memchr_lookup);
    }

    #[test]
    fn test_scan_simple() {
        test_scan_algo(scan_simple);
    }

    #[test]
    fn test_scan_simple_batched() {
        test_scan_algo(scan_simple_batched);
    }

    #[test]
    fn test_scan_simple_batched_lookup() {
        test_scan_algo(scan_simple_batched_lookup);
    }

    fn test_scan_algo(scan: ScanFn) {
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
    fn fsd() {
        let data = std::fs::read("games/FSD/FSD-Win64-Shipping.exe").unwrap();

        let raw_patterns = crate::patterns::get_patterns()
            .unwrap()
            .into_iter()
            .map(|c| c.pattern)
            .collect::<Vec<_>>();
        let id_patterns = raw_patterns.iter().map(|p| (&(), p)).collect::<Vec<_>>();

        let p = id_patterns;
        scan(&p, 0, &data);
        scan_memchr(&p, 0, &data);
        scan_simple(&p, 0, &data);
        scan_simple_batched(&p, 0, &data);
        scan_simple_batched_lookup(&p, 0, &data);
    }
}
