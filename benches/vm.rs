//! VM benchmarks: `cargo bench`. Each program is compiled and run from scratch
//! per iteration, so startup cost is included (it is ~a few µs).

use criterion::{Criterion, criterion_group, criterion_main};
use dog4::{OutputBuffer, Runtime};

const PROGRAMS: &[(&str, &str)] = &[
    (
        "fib_20",
        "fn fib (n) { if ($n < 2) { return $n }; put (fib($n - 1) + fib($n - 2)) } put { fib 20 }",
    ),
    (
        "fib_20_dog3_style",
        "fn fib (n) { if { lt $n 2 } { return $n }; add { fib { sub $n 1 } } { fib { sub $n 2 } } } put { fib 20 }",
    ),
    (
        "for_sum_100k",
        "s = 0; for i in (0..100000) { s += $i }; put $s",
    ),
    (
        "while_sum_100k",
        "fn f () { i = 0; s = 0; while ($i < 100000) { s += $i; i += 1 }; put $s } put { f }",
    ),
    (
        "collect_array_50k",
        "xs = for i in (0..50000) { put [$i] }; put { len $xs }",
    ),
    (
        "build_string_20k",
        "s = for i in (0..20000) { put \"$i,\" }; put { strlen $s }",
    ),
    (
        "map_merge_5k",
        "m = for i in (0..5000) { put [$i = ($i * 2)] }; put { len $m }",
    ),
    (
        "closures_map_filter_10k",
        "xs = { map (0..10000) fn (x) { put ($x * 3) } }; put { len { filter $xs fn (x) { put ($x % 2 == 0) } } }",
    ),
    (
        "references_push_20k",
        "fn push (l: &arr, x) { l += [$x] } fn f () { xs = []; for i in (0..20000) { push &xs $i }; put { len $xs } } put { f }",
    ),
    (
        "mergesort_dog3",
        include_str!("../tests/dog3/mergesort.dog"),
    ),
    (
        "json_round_trip",
        "data = for i in (0..2000) { put [[id = $i, tags = [a b], ok = true]] }; put { len (json.parse(json.encode($data))) }",
    ),
];

fn bench(c: &mut Criterion) {
    for (name, source) in PROGRAMS {
        c.bench_function(name, |b| {
            b.iter(|| {
                let mut rt = Runtime::builder().output(OutputBuffer::default()).build();
                rt.run("bench", source).expect("benchmark program failed")
            })
        });
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = bench
}
criterion_main!(benches);
