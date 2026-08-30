//! Protocol encode/decode benchmarks.
//!
//! The protocol runs once per file and once per response, so its cost is
//! multiplied by the corpus size. These measure the per-message cost that a
//! million-file crawl pays a million times.

use crawl_domain::ids::RequestId;
use crawl_protocol::{decode_response, encode_request, Request};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn response_line(rows: usize) -> String {
    let records: Vec<String> = (0..rows)
        .map(|index| {
            format!(
                r#"{{"filename":"/data/corpus/subdir/file-{index}.txt","line":{index},"entity":"Example Entity {index}"}}"#
            )
        })
        .collect();
    format!(
        r#"{{"id":42,"status":"ok","rows":[{}]}}"#,
        records.join(",")
    )
}

fn encode(c: &mut Criterion) {
    let request = Request::process(RequestId::new(42), "/data/corpus/subdir/file-1234.txt");
    c.bench_function("encode_request", |b| {
        b.iter(|| encode_request(black_box(&request)).expect("encode"));
    });
}

fn decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_response");
    for rows in [0usize, 1, 10, 100] {
        let line = response_line(rows);
        group.throughput(Throughput::Bytes(line.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(rows), &line, |b, line| {
            b.iter(|| decode_response(black_box(line)).expect("decode"));
        });
    }
    group.finish();
}

fn decode_failure(c: &mut Criterion) {
    // The rejection path runs on every malformed line, so it must not be
    // pathologically slower than the success path.
    c.bench_function("decode_response_malformed", |b| {
        b.iter(|| decode_response(black_box("this is not json at all")).unwrap_err());
    });
}

criterion_group!(benches, encode, decode, decode_failure);
criterion_main!(benches);
