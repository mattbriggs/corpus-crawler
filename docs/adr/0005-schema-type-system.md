# ADR 0005: A four-type, flat schema language

**Status:** Accepted · **Covers:** REQ-091 - REQ-094, VAL-020 - VAL-026, OQ-022

## Context

CSV is tabular; Python objects are not. The schema language decides what a
plugin may promise and what the host will accept.

## Decision

Four scalar types—`string`, `integer`, `number`, `boolean`—and flat records
only. `required: true` means "present and non-null"; the default `false` means
"may be absent or null". That single convention satisfies REQ-156.

`datetime` is **excluded**. REQ-092 makes it optional and conditions it on
defining a canonical lexical and CSV representation, which the SRS does not do.

## Consequences

- No lossy coercion (VAL-026). The one widening rule is documented and tested:
  `integer` accepts a JSON float with a zero fractional part, because encoders
  legitimately render `7` as `7.0`. Strings are never parsed into other types.
- Nested arrays and objects are rejected with the offending field named
  (AC-028). A plugin that needs structure serializes it into a `string` field.
- Column order is declaration order, so records arriving with differently
  ordered keys still produce identical CSV rows (AC-011).
- Duplicate field names are a manifest parse error, which required a custom
  map visitor: an ordinary YAML map would have silently dropped the duplicate
  and made VAL-020 unenforceable.
