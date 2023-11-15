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

    let result = scan_pattern(&[&pattern], 0, &data);
    assert_eq!(result, vec![vec![size - needle.len()]]);

    c.bench_function("gig scan", |b| {
        b.iter(|| scan_pattern(&[&pattern], 0, &data))
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

    let id_patterns = raw_patterns.iter().collect::<Vec<_>>();

    let mut group = c.benchmark_group("xref");

    let t = 5;
    for f in 0..t {
        let size = (raw_patterns.len() as f64 * f as f64 / t as f64).round() as usize;
        let p = &id_patterns[0..size];
        group.bench_with_input(BenchmarkId::new("xref", size), &size, |b, _size| {
            b.iter(|| scan_xref(p, base_address, data))
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

criterion_main!(bench1, bench2);
