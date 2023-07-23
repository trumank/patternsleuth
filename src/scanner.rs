use std::{simd::{SimdPartialEq, ToBitMask}, collections::HashMap};

use itertools::Itertools;

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

#[derive(Debug, Clone, Copy, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Xref(pub usize);

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
                    if let Some(address) = (base_address + width + j)
                        .checked_add_signed(
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

    let patterns = patterns.iter().sorted_by_key(|p| p.1).collect::<Vec<_>>();

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
                    if let Some(address) = (base_address + width + j)
                        .checked_add_signed(
                            i32::from_le_bytes(data[j..j + width].try_into().unwrap())
                                .try_into()
                                .unwrap(),
                        ) {

                        if let Ok(i) = patterns.binary_search_by_key(&address, |p| p.1.0) {
                            matches.push((patterns[i].0, base_address + j));
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

    let patterns = patterns.iter().map(|(id, p)| (p.0, *id)).collect::<HashMap<_, _>>();

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
                    if let Some(address) = (base_address + width + j)
                        .checked_add_signed(
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
    fn test_scan_memchr() {
        test_scan_algo(scan_memchr);
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
    fn test_scan_xref() {
        let patterns = [(&(), &Xref(10))];
        dbg!(scan_xref(&patterns, 0, &[0; 100]));
    }
}
