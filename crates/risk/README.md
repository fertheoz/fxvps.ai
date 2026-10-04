# risk

Account groups (`GroupConfig`: leverage, hedging/netting, margin-call and
stop-out levels, volume/exposure limits, symbol permissions, markup, price
tolerance, hedged margin %, ESMA preset, negative balance protection, A/B-book
routing), symbol specs (`SymbolSpec`), `QuoteBook` with direct/inverse/USD-cross
conversion, margin (`symbol_margin`, `total_margin`, hedged legs charged at
`hedged_margin_bps` each), P&L, `pre_trade_check`, `AccountRisk` levels,
`liquidation_order` (largest loss first) and `negative_balance_compensation`.

Margin rounds up (Ceiling) and P&L rounds half-even. Proptests: margin is
monotonic in unhedged volume and leverage, and hedged margin is sub-additive.
