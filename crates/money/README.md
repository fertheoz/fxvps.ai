# money

Fixed-point monetary types: `Price`/`Qty` (i64, scale 1e8), `Money` (i128 minor
units + `Currency`), explicit `Rounding` modes (half-even, half-up, down, up,
floor, ceiling), checked arithmetic and single-rounding currency conversion.
No floats anywhere. All types are serde-serializable (`Currency` as a string).

## Relation to `domain::Fixed` (M1)

`domain::Fixed` (M1) is an untyped i64 with the same 1e8 scale. `money` keeps
distinct `Price` and `Qty` newtypes (so a quantity cannot be passed as a price)
plus a currency-tagged `Money`, which `Fixed` does not model. Conversions are
lossless `From` impls in both directions (`Price <-> Fixed`, `Qty <-> Fixed`),
so FIX-gateway values flow into the engine without rescaling.
