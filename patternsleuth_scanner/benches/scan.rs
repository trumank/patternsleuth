use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use patternsleuth_scanner::*;

fn gig(c: &mut Criterion) {
    use rand::prelude::*;
    let size = 1024 * 1024 * 1024;
    let mut data: Vec<u8> = Vec::with_capacity(size);
    let mut rng = rand::thread_rng();

    let needle = b"\xf9\x82\xdb\xdb\x2d\x32\x6f\x15\x11\x44\x54\xf4\xc8\xaa\xd1\x72\x53\x96\xa5\x7b\x22\x24\x94\x7f\xec\x28\xc7\xe0\x5e\xd4\xae\x39";

    data.extend((0..size - needle.len()).map(|_| rng.gen::<u8>()));
    data.extend(needle);

    let pattern = Pattern::new("f9 82 db db 2d ?? 6f 15 ?? 44 54 f4 c8 aa d1 72 53 ?? a5 7b 22 24 94 7f ec 28 ?? e0 5e d4 ae 39").unwrap();

    let result = scan(&[(&(), &pattern)], 0, &data);
    assert_eq!(result, [(&(), size - needle.len())]);

    let result = scan_memchr(&[(&(), &pattern)], 0, &data);
    assert_eq!(result, [(&(), size - needle.len())]);

    c.bench_function("gig scan", |b| {
        b.iter(|| scan(&[(&(), &pattern)], 0, &data))
    });
    c.bench_function("gig scan_memchr", |b| {
        b.iter(|| scan_memchr(&[(&(), &pattern)], 0, &data))
    });
    c.bench_function("gig scan_memchr_lookup", |b| {
        b.iter(|| scan_memchr_lookup(&[(&(), &pattern)], 0, &data))
    });
    c.bench_function("gig scan_memchr_lookup_many", |b| {
        b.iter(|| scan_memchr_lookup_many(&[(&(), &pattern)], 0, &data))
    });
}

fn many(c: &mut Criterion) {
    use object::Object;
    use object::ObjectSection;

    let bin_data = std::fs::read("../games/FSD/FSD-Win64-Shipping.exe").unwrap();
    let obj_file = object::File::parse(&*bin_data).unwrap();
    let section = obj_file.section_by_name(".text").unwrap();
    let data = section.data().unwrap();

    let patterns = include_str!("patterns.txt")
        .lines()
        .map(|l| Pattern::new(l).unwrap())
        .collect::<Vec<_>>();

    let indexed_patterns = patterns.iter().map(|p| (&(), p)).collect::<Vec<_>>();

    let mut reference = scan(&indexed_patterns, 0, data);
    reference.sort();

    let mut next = scan_memchr(&indexed_patterns, 0, data);
    next.sort();
    assert_eq!(&next, &reference);

    let mut next = scan_memchr_lookup(&indexed_patterns, 0, data);
    next.sort();
    assert_eq!(&next, &reference);

    let mut next = scan_memchr_lookup_many(&indexed_patterns, 0, data);
    next.sort();
    assert_eq!(&next, &reference);

    c.bench_function("many scan", |b| b.iter(|| scan(&indexed_patterns, 0, data)));
    c.bench_function("many scan_memchr", |b| {
        b.iter(|| scan_memchr(&indexed_patterns, 0, data))
    });
    c.bench_function("many scan_memchr_lookup", |b| {
        b.iter(|| scan_memchr_lookup(&indexed_patterns, 0, data))
    });
    c.bench_function("many scan_memchr_lookup_many", |b| {
        b.iter(|| scan_memchr_lookup_many(&indexed_patterns, 0, data))
    });
}

fn xref(c: &mut Criterion) {
    use object::Object;
    use object::ObjectSection;

    let bin_data = std::fs::read("../games/FSD/FSD-Win64-Shipping.exe").unwrap();
    let obj_file = object::File::parse(&*bin_data).unwrap();
    let section = obj_file.section_by_name(".text").unwrap();
    let base_address = section.address() as usize;
    let data = section.data().unwrap();

    let raw_patterns = [
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x146CAC280),
        Xref(0x146CAC288),
        Xref(0x141DBABA0),
        Xref(0x1450BB188),
        Xref(0x1450BB1A8),
        Xref(0x1450BB378),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
        Xref(0x1450BB398),
        Xref(0x144F4DA28),
        Xref(0x144F4DA40),
        Xref(0x144F4D6D8),
    ];

    let id_patterns = raw_patterns.iter().map(|p| (&(), p)).collect::<Vec<_>>();

    let mut group = c.benchmark_group("xref");

    let t = 5;
    for f in 0..t {
        let size = (raw_patterns.len() as f64 * f as f64 / t as f64).round() as usize;
        let p = &id_patterns[0..size];
        group.bench_with_input(BenchmarkId::new("xref_linear", size), &size, |b, _size| {
            b.iter(|| scan_xref(p, base_address, data))
        });
        group.bench_with_input(BenchmarkId::new("xref_binary", size), &size, |b, _size| {
            b.iter(|| scan_xref_binary(p, base_address, data))
        });
        group.bench_with_input(BenchmarkId::new("xref_hash", size), &size, |b, _size| {
            b.iter(|| scan_xref_hash(p, base_address, data))
        });
    }

    group.finish();
}

criterion_group! {
    name = bench1;
    config = Criterion::default().sample_size(30);
    targets = gig
}
criterion_group!(bench2, xref);
criterion_group!(bench3, many);

criterion_main!(bench1, bench2, bench3);
