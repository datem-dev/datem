# Stripe Billing Integration

> How datem maps usage events to Stripe customers, meters, and invoices.

Datem is not a payment processor. It is a metering and billing engine. Stripe handles payment collection, invoicing, and tax. Datem handles event ingestion, usage aggregation, and telling Stripe what to charge.

---

## Mental Model

```
your app
   │
   │  POST /ingest  (usage events)
   ▼
datem
   │
   ├── aggregates usage per customer per billing period
   │
   └── at period close:
        ├── reports usage to Stripe Meter
        └── Stripe generates and sends invoice
```

Datem never touches payment details, card numbers, or bank accounts. Stripe owns the money movement. Datem owns the source of truth for what was consumed.

---

## Core Concepts

### Datem Customer ↔ Stripe Customer

Every customer in datem maps 1:1 to a Stripe Customer object. Datem stores the `stripe_customer_id` on its own customer record so it can report usage and create invoice items against the right Stripe Customer.

```
datem customers table
┌─────────────────┬──────────────────────┬─────────────────────┐
│ id              │ name                 │ stripe_customer_id  │
├─────────────────┼──────────────────────┼─────────────────────┤
│ cust_abc123     │ Acme Corp            │ cus_StripeXYZ789    │
└─────────────────┴──────────────────────┴─────────────────────┘
```

The Stripe Customer can be created two ways:

**A. Datem creates it** — when you add a customer to datem, datem calls `POST /v1/customers` on Stripe and stores the returned `id`.

**B. You bring your own** — if the customer already exists in Stripe (e.g. from an existing Stripe Billing subscription), you pass `stripe_customer_id` when creating the datem customer record. No duplicate is created.

---

## Stripe Billing Modes

Datem supports two Stripe billing modes. Choose based on how you want invoicing to work.

### Mode 1: Stripe Meters (recommended)

Stripe's native usage metering product. Datem reports aggregated usage to a Stripe Meter at period close. Stripe handles proration, invoice generation, and sending.

Best for: straightforward per-unit and tiered pricing where Stripe's pricing model covers your needs.

### Mode 2: Invoice Items (escape hatch)

Datem aggregates usage itself, calculates the charge, and creates a Stripe Invoice Item directly. Datem then finalises the invoice.

Best for: complex pricing logic (hybrid plans, custom credits, negotiated rates) where Stripe's meter pricing is too rigid.

MVP ships both. Mode 1 is the default.

---

## Worked Example: LLM Credits

A common SaaS pattern — customers purchase or consume LLM tokens, charged at the end of the billing period.

### 1. Define the metric in datem

```toml
# datem.toml
[[metrics]]
id          = "llm_tokens"
display     = "LLM Tokens"
aggregation = "sum"          # sum all tokens in the period
```

### 2. Define the plan

```toml
[[plans]]
id       = "pro"
name     = "Pro"
currency = "usd"
interval = "monthly"

[[plans.charges]]
metric            = "llm_tokens"
model             = "tiered"
stripe_meter_id   = "mtr_xxxxxxxxxx"   # created once in Stripe dashboard or via API

tiers = [
  { up_to = 1_000_000,  unit_price = 0.000002 },   # $2 per 1M tokens, first 1M
  { up_to = 10_000_000, unit_price = 0.0000015 },   # $1.50 per 1M thereafter
  { up_to = null,       unit_price = 0.000001  },   # $1 per 1M at scale
]
```

### 3. Create the Stripe Meter (once, at setup)

Datem does this automatically when you define a metric with `stripe_meter_id` not yet set, or you can run:

```bash
datem stripe meters create --metric llm_tokens
```

Which calls:

```
POST /v1/billing/meters
{
  "display_name": "LLM Tokens",
  "event_name": "datem_llm_tokens",
  "default_aggregation": { "formula": "sum" },
  "customer_mapping": {
    "event_payload_key": "stripe_customer_id",
    "type": "by_id"
  },
  "value_settings": { "event_payload_key": "quantity" }
}
```

### 4. Create a Stripe subscription for the customer

When a customer subscribes to the Pro plan, datem:

1. Looks up (or creates) the Stripe Customer
2. Creates a Stripe Subscription with the meter-based price:

```
POST /v1/subscriptions
{
  "customer": "cus_StripeXYZ789",
  "items": [{
    "price": "price_xxxxxxxxxx"   // the metered price linked to mtr_xxxxxxxxxx
  }],
  "billing_cycle_anchor": 1718918400,
  "collection_method": "send_invoice",
  "days_until_due": 30
}
```

Datem stores the `stripe_subscription_id` on the subscription record.

### 5. Ingest usage events

Your application sends events to datem throughout the billing period:

```bash
curl -X POST https://your-datem/ingest \
  -H "Authorization: Bearer $DATEM_API_KEY" \
  -d '{
    "event_id":    "01HWXYZ001",
    "customer_id": "cust_abc123",
    "metric":      "llm_tokens",
    "quantity":    4096,
    "timestamp":   1718918400000000,
    "properties": {
      "model":  "gpt-4o",
      "prompt_tokens":     3200,
      "completion_tokens": 896
    }
  }'
```

Events accumulate in Tonbo. The `properties` field is stored and queryable but does not affect billing — only `metric` and `quantity` feed the billing engine.

### 6. Period close — reporting to Stripe

At the end of the billing period (or on the configured flush interval), datem's billing engine runs:

**Step 1 — Aggregate usage from Tonbo**

```sql
SELECT
    customer_id,
    SUM(quantity) AS total_tokens
FROM events
WHERE metric    = 'llm_tokens'
  AND timestamp >= period_start
  AND timestamp <  period_end
GROUP BY customer_id
```

**Step 2 — Report to Stripe Meter**

For each customer with usage, datem calls:

```
POST /v1/billing/meter_events
{
  "event_name": "datem_llm_tokens",
  "payload": {
    "stripe_customer_id": "cus_StripeXYZ789",
    "quantity":           "8388608"
  },
  "timestamp": 1721596799   // last second of billing period
}
```

Stripe receives this, applies it to the active subscription, and generates the invoice automatically.

**Step 3 — Record the billing run**

Datem writes a `billing_runs` record to Tonbo:

```
billing_runs table
┌──────────────┬─────────────┬──────────────┬────────────┬────────┐
│ id           │ customer_id │ period_start │ period_end │ status │
├──────────────┼─────────────┼──────────────┼────────────┼────────┤
│ run_abc001   │ cust_abc123 │ 1718918400   │ 1721596799 │ done   │
└──────────────┴─────────────┴──────────────┴────────────┴────────┘
```

This ensures idempotency — if datem restarts mid-run, it checks `billing_runs` before re-reporting. A completed run for a customer+period is never reported twice.

---

## Customer Linking API

### Create a customer (datem creates Stripe Customer)

```bash
curl -X POST https://your-datem/customers \
  -d '{
    "id":    "cust_abc123",
    "name":  "Acme Corp",
    "email": "billing@acme.com"
  }'
```

Datem calls Stripe, stores `stripe_customer_id` internally. Returns:

```json
{
  "id":                  "cust_abc123",
  "name":                "Acme Corp",
  "stripe_customer_id":  "cus_StripeXYZ789",
  "created_at":          1718918400
}
```

### Create a customer (bring your own Stripe Customer)

```bash
curl -X POST https://your-datem/customers \
  -d '{
    "id":                  "cust_abc123",
    "name":                "Acme Corp",
    "email":               "billing@acme.com",
    "stripe_customer_id":  "cus_StripeXYZ789"
  }'
```

Datem skips the Stripe API call and links directly.

### Look up a customer's Stripe portal URL

```bash
curl https://your-datem/customers/cust_abc123/portal
```

Returns a Stripe Billing Portal session URL so customers can manage their own payment methods and download invoices — without you building any billing UI.

---

## Idempotency

Every Stripe API call datem makes includes an `Idempotency-Key` header derived from the `billing_run_id` + `customer_id` + `period`. This means:

- Retrying a failed billing run never double-bills a customer
- Stripe deduplicates on its end using the same key
- Datem's `billing_runs` table provides a second layer of deduplication before any Stripe call is made

---

## Mode 2: Invoice Items (manual aggregation)

For pricing too complex for Stripe Meters — negotiated rates, credit bundles, multi-metric hybrid plans — datem aggregates and prices internally, then pushes a line-item invoice to Stripe.

```
datem billing engine
   │
   ├── runs pricing logic (tiered, hybrid, credits)
   ├── calculates amount in cents
   │
   └── POST /v1/invoiceitems  (one per line item)
       POST /v1/invoices      (finalise + send)
```

The invoice is created in Stripe with `auto_advance: true` so Stripe handles sending, PDF generation, and payment collection automatically.

This mode is configured per-plan:

```toml
[[plans]]
id            = "enterprise"
billing_mode  = "invoice_items"   # datem prices, Stripe invoices
```

---

## Configuration Reference

```toml
[stripe]
secret_key        = "sk_live_..."          # required
webhook_secret    = "whsec_..."            # for payment status callbacks
default_currency  = "usd"
invoice_days_due  = 30                     # net-30 by default
auto_advance      = true                   # Stripe auto-finalises invoices
meter_flush       = "period_close"         # or "hourly", "daily"
```

---

## Webhook Handling

Datem listens for Stripe webhooks to keep subscription and invoice state in sync:

| Event | Datem action |
|---|---|
| `invoice.paid` | Mark invoice paid in Tonbo |
| `invoice.payment_failed` | Flag customer, trigger retry logic |
| `customer.subscription.deleted` | Mark subscription cancelled |
| `customer.subscription.updated` | Sync plan changes |

Webhook endpoint: `POST /webhooks/stripe`

---

## MVP Scope

| Feature | Status |
|---|---|
| Stripe Customer create / link | ✅ MVP |
| Stripe Meter reporting (Mode 1) | ✅ MVP |
| Stripe Invoice Items (Mode 2) | ✅ MVP |
| Billing run idempotency | ✅ MVP |
| Stripe Billing Portal passthrough | ✅ MVP |
| Webhook handling | ✅ MVP |
| Paddle integration | 🔜 Planned |
| Multi-currency per customer | 🔜 Planned |
| Tax (Stripe Tax) | 🔜 Planned |
| Revenue recognition reporting | 🔜 Planned |