# money

Fixed-point monetary types: `Price`/`Qty` (i64, scale 1e8), `Money` (i128 minor
units + `Currency`), explicit `Rounding` modes (half-even, half-up, down, up,
floor, ceiling), checked arithmetic and single-rounding currency conversion.
No floats anywhere. All types are serde-serializable (`Currency` as a string).
