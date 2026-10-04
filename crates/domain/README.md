# domain

Venue-agnostic types: `Fixed` (i64, 8 decimals — prices and quantities never use `f64`),
`Instrument`, `Quote`, `Order`, `Execution` and the order enums. No I/O, no FIX knowledge.
`Fixed` serializes to JSON as a decimal string to stay exact across languages.
