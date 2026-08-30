//! Schema validation benchmarks.
//!
//! Validation runs once per emitted row, which is the highest-frequency
//! operation in the host: a crawl emitting ten million rows runs this ten
//! million times.

use crawl_domain::record::{RawRecord, RawValue};
use crawl_domain::schema::{ExtraFieldPolicy, FieldType, OutputSchema, SchemaField};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn schema(width: usize) -> OutputSchema {
    let fields = (0..width)
        .map(|index| SchemaField {
            name: format!("field_{index}"),
            r#type: if index % 2 == 0 {
                FieldType::String
            } else {
                FieldType::Integer
            },
            required: index < 2,
        })
        .collect();
    OutputSchema::new(fields).expect("schema")
}

fn record(width: usize) -> RawRecord {
    (0..width)
        .map(|index| {
            let value = if index % 2 == 0 {
                RawValue::String(format!("value {index}"))
            } else {
                RawValue::Integer(index as i64)
            };
            (format!("field_{index}"), value)
        })
        .collect()
}

fn validate(c: &mut Criterion) {
    let mut group = c.benchmark_group("validate_record");
    for width in [3usize, 10, 30] {
        let schema = schema(width);
        let record = record(width);
        group.bench_with_input(BenchmarkId::from_parameter(width), &width, |b, _| {
            b.iter(|| {
                schema
                    .validate(black_box(&record), ExtraFieldPolicy::Reject)
                    .expect("valid")
            });
        });
    }
    group.finish();
}

fn validate_rejecting(c: &mut Criterion) {
    // Rejection must be cheap: a misbehaving plugin can make it the common case.
    let schema = schema(10);
    let mut record = record(10);
    record.insert("undeclared", RawValue::Bool(true));
    c.bench_function("validate_record_rejected", |b| {
        b.iter(|| {
            schema
                .validate(black_box(&record), ExtraFieldPolicy::Reject)
                .unwrap_err()
        });
    });
}

fn csv_rendering(c: &mut Criterion) {
    let schema = schema(10);
    let validated = schema
        .validate(&record(10), ExtraFieldPolicy::Reject)
        .expect("valid");
    c.bench_function("render_csv_fields", |b| {
        b.iter(|| black_box(&validated).to_csv_fields());
    });
}

criterion_group!(benches, validate, validate_rejecting, csv_rendering);
criterion_main!(benches);
