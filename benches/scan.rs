use criterion::{criterion_group, criterion_main, Criterion};

fn gig(c: &mut Criterion) {
    use rand::prelude::*;
    let size = 1024 * 1024 * 1024;
    let mut data: Vec<u8> = Vec::with_capacity(size);
    let mut rng = rand::thread_rng();

    let needle = b"\xf9\x82\xdb\xdb\x2d\x32\x6f\x15\x11\x44\x54\xf4\xc8\xaa\xd1\x72\x53\x96\xa5\x7b\x22\x24\x94\x7f\xec\x28\xc7\xe0\x5e\xd4\xae\x39";

    data.extend((0..size - needle.len()).map(|_| rng.gen::<u8>()));
    data.extend(needle);

    let pattern = patternsleuth::Pattern::new("f9 82 db db 2d ?? 6f 15 ?? 44 54 f4 c8 aa d1 72 53 ?? a5 7b 22 24 94 7f ec 28 ?? e0 5e d4 ae 39").unwrap();

    let result = patternsleuth::scan(&[(&(), &pattern)], 0, &data);
    assert_eq!(result, [(&(), size - needle.len())]);

    c.bench_function("gig", |b| {
        b.iter(|| patternsleuth::scan(&[(&(), &pattern)], 0, &data))
    });
}

criterion_group! {
    name = bench1;
    config = Criterion::default().sample_size(30);
    targets = gig
}

criterion_main!(bench1);
