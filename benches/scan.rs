use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

fn gig(c: &mut Criterion) {
    use rand::prelude::*;
    let size = 1024 * 1024 * 1024;
    let mut data: Vec<u8> = Vec::with_capacity(size);
    let mut rng = rand::thread_rng();

    let needle = b"\xf9\x82\xdb\xdb\x2d\x32\x6f\x15\x11\x44\x54\xf4\xc8\xaa\xd1\x72\x53\x96\xa5\x7b\x22\x24\x94\x7f\xec\x28\xc7\xe0\x5e\xd4\xae\x39";

    data.extend((0..size - needle.len()).map(|_| rng.gen::<u8>()));
    data.extend(needle);

    let pattern = patternsleuth::Pattern::new("f9 82 db db 2d ?? 6f 15 ?? 44 54 f4 c8 aa d1 72 53 ?? a5 7b 22 24 94 7f ec 28 ?? e0 5e d4 ae 39").unwrap();

    let result = patternsleuth::scanner::scan(&[(&(), &pattern)], 0, &data);
    assert_eq!(result, [(&(), size - needle.len())]);

    let result = patternsleuth::scanner::scan_memchr(&[(&(), &pattern)], 0, &data);
    assert_eq!(result, [(&(), size - needle.len())]);

    c.bench_function("one gig scan", |b| {
        b.iter(|| patternsleuth::scanner::scan(&[(&(), &pattern)], 0, &data))
    });
    c.bench_function("one gig scan_memchr", |b| {
        b.iter(|| patternsleuth::scanner::scan_memchr(&[(&(), &pattern)], 0, &data))
    });
    c.bench_function("one gig scan_simple", |b| {
        b.iter(|| patternsleuth::scanner::scan_simple(&[(&(), &pattern)], 0, &data))
    });
}

fn gig_freq(c: &mut Criterion) {
    use rand::prelude::*;
    let size = 1024 * 1024 * 1024 / 10;
    let mut data: Vec<u8> = vec![0; size];
    let mut rng = rand::thread_rng();

    let needle = b"\x01\x82\xdb\xdb\x2d\x32\x6f\x15\x11\x44\x54\xf4\xc8\xaa\xd1\x72\x53\x96\xa5\x7b\x22\x24\x94\x7f\xec\x28\xc7\xe0\x5e\xd4\xae\x39";

    let pattern = patternsleuth::Pattern::new("01 82 db db 2d ?? 6f 15 ?? 44 54 f4 c8 aa d1 72 53 ?? a5 7b 22 24 94 7f ec 28 ?? e0 5e d4 ae 39").unwrap();

    let mut group = c.benchmark_group("freq");

    let steps = 10;
    for freq in 0..steps + 1 {
        let f = freq as f64 / steps as f64;
        for d in data.iter_mut() {
            *d = if rng.gen_bool(f) { 1 } else { 2 }
        }

        data[(size - needle.len())..].copy_from_slice(needle);

        let result = patternsleuth::scanner::scan(&[(&(), &pattern)], 0, &data);
        assert_eq!(result, [(&(), size - needle.len())]);

        let result = patternsleuth::scanner::scan_memchr(&[(&(), &pattern)], 0, &data);
        assert_eq!(result, [(&(), size - needle.len())]);

        group.bench_with_input(BenchmarkId::new("ps", f), &f, |b, _size| {
            b.iter(|| patternsleuth::scanner::scan(&[(&(), &pattern)], 0, &data))
        });
        group.bench_with_input(BenchmarkId::new("memchr", f), &f, |b, _size| {
            b.iter(|| patternsleuth::scanner::scan_memchr(&[(&(), &pattern)], 0, &data))
        });
        group.bench_with_input(BenchmarkId::new("simple", f), &f, |b, _size| {
            b.iter(|| patternsleuth::scanner::scan_simple(&[(&(), &pattern)], 0, &data))
        });

        /*
        c.bench_function(&format!("freq gig scan = {:.2}", f), |b| {
            b.iter(|| patternsleuth::scanner::scan(&[(&(), &pattern)], 0, &data))
        });
        c.bench_function(&format!("freq gig scan_memchr = {:.2}", f), |b| {
            b.iter(|| patternsleuth::scanner::scan_memchr(&[(&(), &pattern)], 0, &data))
        });
        c.bench_function(&format!("freq gig scan_simple = {:.2}", f), |b| {
            b.iter(|| patternsleuth::scanner::scan_simple(&[(&(), &pattern)], 0, &data))
        });
        */
    }
    group.finish();
}

fn multi(c: &mut Criterion) {
    use rand::prelude::*;
    let size = 1024 * 1024 * 1024 / 10;
    let mut data: Vec<u8> = vec![0; size];
    let mut rng = rand::thread_rng();

    let patterns = (1..30)
        .map(|i| {
            //patternsleuth::Pattern::new(&format!("{i:02x} 82 db db 2d ?? 6f 15 ?? 44 54 f4 c8"))
            patternsleuth::Pattern::new(&format!("01 82 db db 2d ?? 6f 15 ?? 44 54 f4 c8")).unwrap()
        })
        .collect::<Vec<_>>();
    let pattern_counts = (1..patterns.len())
        .map(|i| {
            patterns
                .iter()
                .take(i)
                .map(|p| (&(), p))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mut group = c.benchmark_group("multi");
    //group.sampling_mode(SamplingMode::Flat);

    //let steps = 10;
    //for freq in 0..steps + 1 {
    //let f = freq as f64 / steps as f64;
    let f = 0.1;
    for d in data.iter_mut() {
        //*d = if rng.gen_bool(f) { rng.gen_range(1..30) } else { 0 }
        *d = if rng.gen_bool(f) { 1 } else { 0 }
    }

    for p in pattern_counts {
        let size = p.len();
        group.bench_with_input(
            BenchmarkId::new("patternsleuth", size),
            &size,
            |b, _size| b.iter(|| patternsleuth::scanner::scan(&p, 0, &data)),
        );
        group.bench_with_input(BenchmarkId::new("memchr", size), &size, |b, _size| {
            b.iter(|| patternsleuth::scanner::scan_memchr(&p, 0, &data))
        });
        group.bench_with_input(BenchmarkId::new("simple", size), &size, |b, _size| {
            b.iter(|| patternsleuth::scanner::scan_simple(&p, 0, &data))
        });
    }

    /*
    c.bench_function(&format!("freq gig scan = {:.2}", f), |b| {
        b.iter(|| patternsleuth::scanner::scan(&[(&(), &pattern)], 0, &data))
    });
    c.bench_function(&format!("freq gig scan_memchr = {:.2}", f), |b| {
        b.iter(|| patternsleuth::scanner::scan_memchr(&[(&(), &pattern)], 0, &data))
    });
    c.bench_function(&format!("freq gig scan_simple = {:.2}", f), |b| {
        b.iter(|| patternsleuth::scanner::scan_simple(&[(&(), &pattern)], 0, &data))
    });
    */
    //}
    group.finish();
}

fn fsd(c: &mut Criterion) {
    let data = std::fs::read("games/FSD/FSD-Win64-Shipping.exe").unwrap();

    let raw_patterns = patternsleuth::patterns::get_patterns()
        .unwrap()
        .into_iter()
        .map(|c| c.pattern)
        .collect::<Vec<_>>();
    let id_patterns = raw_patterns.iter().map(|p| (&(), p)).collect::<Vec<_>>();

    let mut group = c.benchmark_group("fsd");

    let t = 5;
    for f in 0..t {
        let size = (raw_patterns.len() as f64 * f as f64 / t as f64).round() as usize;
        let p = &id_patterns[0..size];
        group.bench_with_input(
            BenchmarkId::new("patternsleuth", size),
            &size,
            |b, _size| b.iter(|| patternsleuth::scanner::scan(&p, 0, &data)),
        );
        group.bench_with_input(
            BenchmarkId::new("patternsleuth_lookup", size),
            &size,
            |b, _size| b.iter(|| patternsleuth::scanner::scan_lookup(&p, 0, &data)),
        );
        group.bench_with_input(BenchmarkId::new("memchr", size), &size, |b, _size| {
            b.iter(|| patternsleuth::scanner::scan_memchr(&p, 0, &data))
        });
        group.bench_with_input(
            BenchmarkId::new("memchr_lookup", size),
            &size,
            |b, _size| b.iter(|| patternsleuth::scanner::scan_memchr_lookup(&p, 0, &data)),
        );
        group.bench_with_input(BenchmarkId::new("simple", size), &size, |b, _size| {
            b.iter(|| patternsleuth::scanner::scan_simple(&p, 0, &data))
        });
        group.bench_with_input(
            BenchmarkId::new("simple_batched", size),
            &size,
            |b, _size| b.iter(|| patternsleuth::scanner::scan_simple_batched(&p, 0, &data)),
        );
        group.bench_with_input(
            BenchmarkId::new("simple_batched_lookup", size),
            &size,
            |b, _size| b.iter(|| patternsleuth::scanner::scan_simple_batched_lookup(&p, 0, &data)),
        );
    }

    group.finish();
}

/*
criterion_group! {
    name = bench1;
    config = Criterion::default().sample_size(30);
    targets = gig
}
criterion_group! {
    name = bench2;
    config = Criterion::default().sample_size(30);
    targets = gig_freq
}
criterion_main!(bench1, bench2);
*/

criterion_group!(bench1, gig_freq);
criterion_group!(bench2, multi);
criterion_group!(bench3, fsd);
criterion_main!(bench1, bench2, bench3);
