# Custom Finance Bot Guide

Use this guide to customize the AI's behavior when processing trade transactions and answering portfolio questions.

## Core Directives

1. **Precision**: Always extract exact quantities and transaction prices.
2. **Standardization**:
   - Symbols must always be stored in **UPPERCASE** (e.g. `AAPL`, `BTC`, `EURUSD`).
   - Format transaction actions clearly in the ledger as `BUY`, `SELL`, `OPEN LONG`, `CLOSE LONG`, `OPEN SHORT`, or `CLOSE SHORT`.
3. **Ledger Table**:
   - Maintain a Markdown table with columns: `Date | Action | Price | Quantity | Realized PnL | Notes`
   - Keep entries strictly ordered by date-time.
4. **Calculations**:
   - When a transaction is recorded, update the frontmatter fields dynamically:
     - `status`: Set to `open` if position size > 0, otherwise `closed`.
     - `position_size`: Current net shares or contracts held.
     - `average_entry`: Calculate the moving weighted average of the open position. When completely closed, reset this to `0`.
     - `realized_profit`: Accumulate realized profits/losses from close operations (in USD or the asset's quote currency).
5. **Photo Links**:
   - When an image link is provided (e.g., `![[Finance/Assets/filename.jpg]]`), append it as an inline link inside the transaction's notes column, or list it clearly under a dedicated notes heading.
