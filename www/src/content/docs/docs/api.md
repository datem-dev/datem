---
title: API Reference
description: datem HTTP API reference — metrics, plans, customers, subscriptions, events, billing runs, invoices, and webhooks.
---


> Version: 0.1 · Base URL: `https://your-datem` · All requests/responses are JSON

Authentication uses bearer tokens. Pass your API key on every request:

```
Authorization: Bearer $DATEM_API_KEY
```

All timestamps are Unix microseconds (µs). All monetary amounts are in the smallest currency unit (cents for USD). All IDs are client-supplied strings — use ULIDs or UUIDs for guaranteed uniqueness.

---

## Table of Contents

- [Metrics](#metrics)
- [Plans](#plans)
- [Customers](#customers)
- [Subscriptions](#subscriptions)
- [Events](#events)
- [Query](#query)
- [Billing Runs](#billing-runs)
- [Invoices](#invoices)
- [Webhooks](#webhooks)
- [Errors](#errors)

---

## Metrics

A metric defines a billable unit of measurement. Metrics must be defined before they can be referenced in a plan charge.

### Object

```json
{
  "id":          "llm_tokens",
  "display":     "LLM Tokens",
  "aggregation": "sum",
  "created_at":  1718918400000000
}
```

| Field | Type | Description |
|---|---|---|
| `id` | string | Unique identifier. Snake case recommended. |
| `display` | string | Human-readable name shown on invoices. |
| `aggregation` | enum | How events are aggregated over a period. One of `sum`, `count`, `max`, `unique_count`. |
| `created_at` | int | Unix microseconds. |

**Aggregation types**

| Value | Description | Example use case |
|---|---|---|
| `sum` | Sum of all `quantity` values in the period | Tokens, API calls, GB transferred |
| `count` | Number of events regardless of quantity | Seats, deploys, builds |
| `max` | Highest single `quantity` value in the period | Peak memory, max concurrent users |
| `unique_count` | Count of distinct values in `properties.key` | Monthly active users |

---

### POST /metrics

Create a metric.

**Request**

```bash
curl -X POST https://your-datem/metrics \
  -H "Authorization: Bearer $DATEM_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "id":          "llm_tokens",
    "display":     "LLM Tokens",
    "aggregation": "sum"
  }'
```

**Response** `201 Created`

```json
{
  "id":          "llm_tokens",
  "display":     "LLM Tokens",
  "aggregation": "sum",
  "created_at":  1718918400000000
}
```

---

### GET /metrics

List all metrics.

**Response** `200 OK`

```json
{
  "data": [
    {
      "id":          "llm_tokens",
      "display":     "LLM Tokens",
      "aggregation": "sum",
      "created_at":  1718918400000000
    },
    {
      "id":          "api_calls",
      "display":     "API Calls",
      "aggregation": "count",
      "created_at":  1718918400000000
    }
  ]
}
```

---

### GET /metrics/{metric_id}

Get a single metric.

**Response** `200 OK` — metric object.

---

### DELETE /metrics/{metric_id}

Archive a metric. Metrics referenced by an active plan charge cannot be archived.

**Response** `204 No Content`

---

## Plans

A plan defines a set of charges applied to a customer over a billing interval. Plans are immutable once created — to change pricing, create a new plan version and migrate subscriptions.

### Object

```json
{
  "id":       "pro-v1",
  "name":     "Pro",
  "status":   "active",
  "currency": "usd",
  "interval": "monthly",
  "charges": [
    {
      "id":         "charge_001",
      "metric":     "llm_tokens",
      "model":      "tiered",
      "tiers": [
        { "up_to": 1000000,  "unit_price": 0.000002  },
        { "up_to": 10000000, "unit_price": 0.0000015 },
        { "up_to": null,     "unit_price": 0.000001  }
      ]
    },
    {
      "id":         "charge_002",
      "metric":     "api_calls",
      "model":      "per_unit",
      "unit_price": 0.0001
    },
    {
      "id":          "charge_003",
      "metric":      null,
      "model":       "flat",
      "amount":      4900,
      "display":     "Pro base fee"
    }
  ],
  "created_at": 1718918400000000
}
```

**Plan fields**

| Field | Type | Description |
|---|---|---|
| `id` | string | Unique identifier. Include a version suffix e.g. `pro-v1`. |
| `name` | string | Display name. |
| `status` | enum | `active` or `archived`. |
| `currency` | string | ISO 4217 currency code. |
| `interval` | enum | `monthly` or `annual`. |
| `charges` | array | One or more charge objects. |

**Charge models**

| Model | Required fields | Description |
|---|---|---|
| `flat` | `amount` | Fixed charge per billing period, not tied to usage. |
| `per_unit` | `metric`, `unit_price` | Charge per unit of usage. |
| `tiered` | `metric`, `tiers` | Different unit price per usage tier. |
| `package` | `metric`, `package_size`, `package_price` | Charge per block of N units. |
| `hybrid` | `metric`, `flat_amount`, `unit_price` | Flat base fee plus per-unit overage. |

**Tier object**

| Field | Type | Description |
|---|---|---|
| `up_to` | int or null | Upper bound of the tier (inclusive). `null` means unlimited. |
| `unit_price` | float | Price per unit within this tier, in the plan currency. |
| `flat_fee` | float | Optional flat fee applied when this tier is reached. |

---

### POST /plans

Create a plan. Charges and tiers are nested inline.

**Request**

```bash
curl -X POST https://your-datem/plans \
  -H "Authorization: Bearer $DATEM_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "id":       "pro-v1",
    "name":     "Pro",
    "currency": "usd",
    "interval": "monthly",
    "charges": [
      {
        "metric": "llm_tokens",
        "model":  "tiered",
        "tiers": [
          { "up_to": 1000000,  "unit_price": 0.000002  },
          { "up_to": 10000000, "unit_price": 0.0000015 },
          { "up_to": null,     "unit_price": 0.000001  }
        ]
      },
      {
        "metric":     "api_calls",
        "model":      "per_unit",
        "unit_price": 0.0001
      },
      {
        "model":   "flat",
        "amount":  4900,
        "display": "Pro base fee"
      }
    ]
  }'
```

**Response** `201 Created` — full plan object.

---

### GET /plans

List all plans.

**Query parameters**

| Parameter | Type | Description |
|---|---|---|
| `status` | enum | Filter by `active` or `archived`. Default: `active`. |

**Response** `200 OK`

```json
{
  "data": [ /* plan objects */ ]
}
```

---

### GET /plans/{plan_id}

Get a single plan including all charges and tiers.

**Response** `200 OK` — full plan object.

---

### PUT /plans/{plan_id}/archive

Archive a plan. Archived plans accept no new subscriptions. Existing subscriptions continue until cancelled or migrated.

**Response** `200 OK`

```json
{
  "id":     "pro-v1",
  "status": "archived"
}
```

---

### PUT /plans/{plan_id}/migrate

Migrate all active subscriptions on this plan to a new plan version.

**Request**

```json
{
  "to":        "pro-v2",
  "effective": "next_period"
}
```

| Field | Type | Description |
|---|---|---|
| `to` | string | Target plan ID. Must be `active`. |
| `effective` | enum | `next_period` — migration applies at next billing cycle boundary. |

**Response** `200 OK`

```json
{
  "migrated":  42,
  "to":        "pro-v2",
  "effective": "next_period"
}
```

---

## Customers

A customer represents a billable entity. Each customer maps 1:1 to a Stripe Customer.

### Object

```json
{
  "id":                 "cust_abc123",
  "name":               "Acme Corp",
  "email":              "billing@acme.com",
  "stripe_customer_id": "cus_StripeXYZ789",
  "metadata":           { "salesforce_id": "SF-001" },
  "created_at":         1718918400000000
}
```

| Field | Type | Description |
|---|---|---|
| `id` | string | Your internal customer identifier. |
| `name` | string | Customer display name. |
| `email` | string | Billing email. Passed to Stripe. |
| `stripe_customer_id` | string | Stripe Customer ID. Auto-created if omitted. |
| `metadata` | object | Arbitrary key-value pairs. Stored in datem, not sent to Stripe. |

---

### POST /customers

Create a customer. If `stripe_customer_id` is omitted, datem creates a Stripe Customer automatically using `name` and `email`.

**Request**

```bash
# Let datem create the Stripe customer
curl -X POST https://your-datem/customers \
  -H "Authorization: Bearer $DATEM_API_KEY" \
  -d '{
    "id":    "cust_abc123",
    "name":  "Acme Corp",
    "email": "billing@acme.com"
  }'

# Bring your own Stripe customer
curl -X POST https://your-datem/customers \
  -H "Authorization: Bearer $DATEM_API_KEY" \
  -d '{
    "id":                 "cust_abc123",
    "name":               "Acme Corp",
    "email":              "billing@acme.com",
    "stripe_customer_id": "cus_StripeXYZ789"
  }'
```

**Response** `201 Created` — full customer object.

---

### GET /customers

List all customers.

**Query parameters**

| Parameter | Type | Description |
|---|---|---|
| `limit` | int | Max results. Default `20`, max `100`. |
| `after` | string | Cursor for pagination (last `id` from previous page). |

**Response** `200 OK`

```json
{
  "data":    [ /* customer objects */ ],
  "has_more": true,
  "next":    "cust_xyz999"
}
```

---

### GET /customers/{customer_id}

Get a single customer.

**Response** `200 OK` — customer object.

---

### PATCH /customers/{customer_id}

Update a customer's `name`, `email`, or `metadata`. Does not change `stripe_customer_id`.

**Request**

```json
{
  "email": "new-billing@acme.com"
}
```

**Response** `200 OK` — updated customer object.

---

### GET /customers/{customer_id}/portal

Generate a Stripe Billing Portal URL for the customer. Use this to let customers manage payment methods and download invoices without building any billing UI.

**Response** `200 OK`

```json
{
  "url":        "https://billing.stripe.com/session/...",
  "expires_at": 1718922000000000
}
```

URLs expire after 5 minutes.

---

### GET /customers/{customer_id}/usage

Get aggregated usage for a customer across all active metrics for the current billing period.

**Response** `200 OK`

```json
{
  "customer_id":    "cust_abc123",
  "period_start":   1718918400000000,
  "period_end":     1721596799000000,
  "usage": [
    {
      "metric":      "llm_tokens",
      "aggregation": "sum",
      "value":       8388608,
      "updated_at":  1718990000000000
    },
    {
      "metric":      "api_calls",
      "aggregation": "count",
      "value":       14203,
      "updated_at":  1718990000000000
    }
  ]
}
```

---

## Subscriptions

A subscription assigns a customer to a plan and tracks the billing period.

### Object

```json
{
  "id":                    "sub_abc001",
  "customer_id":           "cust_abc123",
  "plan_id":               "pro-v1",
  "status":                "active",
  "current_period_start":  1718918400000000,
  "current_period_end":    1721596799000000,
  "stripe_subscription_id": "sub_StripeABC",
  "created_at":            1718918400000000,
  "cancelled_at":          null
}
```

| Field | Type | Description |
|---|---|---|
| `status` | enum | `active`, `cancelled`, `past_due` |
| `current_period_start` | int | Start of the active billing period (Unix µs). |
| `current_period_end` | int | End of the active billing period (Unix µs). |
| `stripe_subscription_id` | string | Stripe Subscription ID. Created by datem on subscribe. |

---

### POST /subscriptions

Subscribe a customer to a plan. Datem creates the corresponding Stripe Subscription automatically.

**Request**

```bash
curl -X POST https://your-datem/subscriptions \
  -H "Authorization: Bearer $DATEM_API_KEY" \
  -d '{
    "id":          "sub_abc001",
    "customer_id": "cust_abc123",
    "plan_id":     "pro-v1"
  }'
```

**Response** `201 Created` — full subscription object.

---

### GET /subscriptions/{subscription_id}

Get a single subscription.

**Response** `200 OK` — subscription object.

---

### GET /customers/{customer_id}/subscriptions

List all subscriptions for a customer.

**Response** `200 OK`

```json
{
  "data": [ /* subscription objects */ ]
}
```

---

### PUT /subscriptions/{subscription_id}

Change a subscription's plan. The migration is scheduled for the next period boundary — the customer finishes their current period on the existing plan.

**Request**

```json
{
  "plan_id":   "pro-v2",
  "effective": "next_period"
}
```

**Response** `200 OK` — updated subscription object with `plan_id` reflecting the new plan.

---

### DELETE /subscriptions/{subscription_id}

Cancel a subscription. Usage in the current period is still billed at period close.

**Query parameters**

| Parameter | Type | Description |
|---|---|---|
| `effective` | enum | `immediately` or `period_end` (default). |

**Response** `200 OK`

```json
{
  "id":           "sub_abc001",
  "status":       "cancelled",
  "cancelled_at": 1718990000000000
}
```

---

## Events

Usage events are the core input to datem. Every event increments a metric for a customer.

### Object

```json
{
  "event_id":    "01HWXYZ001",
  "customer_id": "cust_abc123",
  "metric":      "llm_tokens",
  "quantity":    4096,
  "timestamp":   1718918400000000,
  "properties": {
    "model":              "gpt-4o",
    "prompt_tokens":      3200,
    "completion_tokens":  896
  }
}
```

| Field | Type | Description |
|---|---|---|
| `event_id` | string | Client-supplied unique ID. Duplicate IDs are silently discarded — use this for safe retries. |
| `customer_id` | string | Must match an existing datem customer. |
| `metric` | string | Must match an existing datem metric. |
| `quantity` | float | The measured value. Interpretation depends on the metric's `aggregation`. |
| `timestamp` | int | When the event occurred (Unix µs). Can be backdated up to 24 hours. |
| `properties` | object | Arbitrary metadata. Stored and queryable via `/query`. Does not affect billing. |

---

### POST /ingest

Ingest a single usage event.

**Request**

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
      "model": "gpt-4o"
    }
  }'
```

**Response** `202 Accepted`

```json
{
  "event_id": "01HWXYZ001",
  "status":   "accepted"
}
```

`202` means the event has been written to Tonbo. It will be included in the next billing run.

---

### POST /ingest/batch

Ingest up to 1,000 events in a single request. Recommended for high-throughput use cases.

**Request**

```bash
curl -X POST https://your-datem/ingest/batch \
  -H "Authorization: Bearer $DATEM_API_KEY" \
  -d '{
    "events": [
      {
        "event_id":    "01HWXYZ001",
        "customer_id": "cust_abc123",
        "metric":      "llm_tokens",
        "quantity":    4096,
        "timestamp":   1718918400000000
      },
      {
        "event_id":    "01HWXYZ002",
        "customer_id": "cust_abc123",
        "metric":      "api_calls",
        "quantity":    1,
        "timestamp":   1718918401000000
      }
    ]
  }'
```

**Response** `202 Accepted`

```json
{
  "accepted": 2,
  "rejected": 0,
  "errors":   []
}
```

Partial success is possible — valid events in a batch are accepted even if others fail validation. Check `errors` for rejected event IDs and reasons.

---

## Query

The query endpoint exposes a DataFusion SQL interface over all datem tables. Use it for ad-hoc analytics, custom reporting, and debugging.

### Available tables

| Table | Description |
|---|---|
| `events` | All ingested usage events |
| `metrics` | Metric definitions |
| `plans` | Plan definitions |
| `charges` | Charge configs per plan |
| `tiers` | Tier configs per charge |
| `customers` | Customer records |
| `subscriptions` | Customer ↔ plan assignments |
| `billing_runs` | Billing engine run log |
| `invoices` | Generated invoices |
| `invoice_line_items` | Line items per invoice |

All tables are automatically scoped to your tenant — you cannot query another tenant's data.

---

### POST /query

Execute a SQL query.

**Request**

```bash
curl -X POST https://your-datem/query \
  -H "Authorization: Bearer $DATEM_API_KEY" \
  -d '{
    "sql": "SELECT customer_id, SUM(quantity) AS tokens FROM events WHERE metric = '\''llm_tokens'\'' GROUP BY customer_id ORDER BY tokens DESC LIMIT 10"
  }'
```

**Response** `200 OK`

```json
{
  "columns": ["customer_id", "tokens"],
  "rows": [
    ["cust_abc123", 8388608],
    ["cust_def456", 4194304]
  ],
  "row_count":    2,
  "elapsed_ms":   42
}
```

**Example queries**

```sql
-- Customers approaching their plan limit this period
SELECT
    s.customer_id,
    SUM(e.quantity)                                  AS used,
    MAX(c.tiers[1].up_to)                            AS first_tier_limit,
    ROUND(SUM(e.quantity) / MAX(c.tiers[1].up_to) * 100, 1) AS pct_used
FROM events e
JOIN subscriptions s ON e.customer_id = s.customer_id
JOIN charges c       ON c.plan_id     = s.plan_id
WHERE e.metric = 'llm_tokens'
GROUP BY 1
HAVING pct_used > 80
ORDER BY pct_used DESC;

-- Revenue by plan this month
SELECT
    s.plan_id,
    COUNT(DISTINCT s.customer_id)  AS customers,
    SUM(i.amount_cents) / 100.0    AS revenue_usd
FROM invoices i
JOIN subscriptions s ON i.subscription_id = s.id
WHERE i.period_start >= EXTRACT(EPOCH FROM DATE_TRUNC('month', NOW())) * 1000000
GROUP BY 1
ORDER BY revenue_usd DESC;

-- Daily usage trend for a customer
SELECT
    DATE_TRUNC('day', TO_TIMESTAMP(timestamp / 1000000)) AS day,
    SUM(quantity)                                         AS tokens
FROM events
WHERE customer_id = 'cust_abc123'
  AND metric      = 'llm_tokens'
GROUP BY 1
ORDER BY 1;
```

---

## Billing Runs

Billing runs are created automatically by the datem billing engine at period close. They are immutable and serve as the idempotency log — a completed run for a customer+period is never re-processed.

### Object

```json
{
  "id":              "run_abc001",
  "customer_id":     "cust_abc123",
  "subscription_id": "sub_abc001",
  "plan_id":         "pro-v1",
  "period_start":    1718918400000000,
  "period_end":      1721596799000000,
  "status":          "completed",
  "invoice_id":      "inv_abc001",
  "created_at":      1721596800000000,
  "completed_at":    1721596812000000
}
```

| Field | Type | Description |
|---|---|---|
| `status` | enum | `pending`, `completed`, `failed` |

---

### GET /billing-runs

List billing runs.

**Query parameters**

| Parameter | Type | Description |
|---|---|---|
| `customer_id` | string | Filter by customer. |
| `status` | enum | Filter by `pending`, `completed`, or `failed`. |
| `limit` | int | Default `20`, max `100`. |

**Response** `200 OK`

```json
{
  "data": [ /* billing run objects */ ]
}
```

---

### GET /billing-runs/{run_id}

Get a single billing run.

**Response** `200 OK` — billing run object.

---

### POST /billing-runs/trigger

Manually trigger a billing run for a specific customer outside the normal schedule. Useful for testing or off-cycle invoicing.

**Request**

```json
{
  "customer_id":  "cust_abc123",
  "period_start": 1718918400000000,
  "period_end":   1721596799000000
}
```

**Response** `202 Accepted`

```json
{
  "run_id": "run_abc002",
  "status": "pending"
}
```

---

## Invoices

Invoices are created by the billing engine after each billing run. They reflect what was reported to Stripe.

### Object

```json
{
  "id":                "inv_abc001",
  "customer_id":       "cust_abc123",
  "subscription_id":   "sub_abc001",
  "billing_run_id":    "run_abc001",
  "stripe_invoice_id": "in_StripeABC",
  "status":            "paid",
  "currency":          "usd",
  "amount_cents":      4250,
  "period_start":      1718918400000000,
  "period_end":        1721596799000000,
  "line_items": [
    {
      "metric":       "llm_tokens",
      "description":  "LLM Tokens — 8,388,608 tokens",
      "quantity":     8388608,
      "amount_cents": 1678,
      "model":        "tiered"
    },
    {
      "metric":       null,
      "description":  "Pro base fee",
      "quantity":     null,
      "amount_cents": 4900,
      "model":        "flat"
    }
  ],
  "created_at": 1721596812000000
}
```

---

### GET /invoices

List invoices.

**Query parameters**

| Parameter | Type | Description |
|---|---|---|
| `customer_id` | string | Filter by customer. |
| `status` | enum | `draft`, `open`, `paid`, `void`. |
| `limit` | int | Default `20`, max `100`. |

**Response** `200 OK`

```json
{
  "data": [ /* invoice objects */ ]
}
```

---

### GET /invoices/{invoice_id}

Get a single invoice including all line items.

**Response** `200 OK` — full invoice object.

---

## Webhooks

Datem emits webhooks on key billing events. Configure your endpoint in the datem settings.

All webhook payloads share a common envelope:

```json
{
  "id":         "evt_abc001",
  "type":       "invoice.paid",
  "created_at": 1721596812000000,
  "data":       { /* event-specific object */ }
}
```

Verify webhook authenticity using the `Datem-Signature` header — a HMAC-SHA256 of the raw request body signed with your webhook secret.

### Event types

| Event | Trigger | Payload |
|---|---|---|
| `invoice.created` | Billing run completes, invoice created | Invoice object |
| `invoice.paid` | Stripe confirms payment | Invoice object |
| `invoice.payment_failed` | Stripe payment attempt fails | Invoice object |
| `subscription.created` | New subscription created | Subscription object |
| `subscription.updated` | Plan change scheduled or applied | Subscription object |
| `subscription.cancelled` | Subscription cancelled | Subscription object |
| `billing_run.failed` | Billing engine run fails | Billing run object |

---

## Errors

All errors follow a consistent shape:

```json
{
  "error": {
    "code":    "customer_not_found",
    "message": "No customer found with id 'cust_abc123'",
    "param":   "customer_id"
  }
}
```

| HTTP status | Meaning |
|---|---|
| `400` | Bad request — missing or invalid parameters |
| `401` | Unauthorized — invalid or missing API key |
| `404` | Not found |
| `409` | Conflict — e.g. duplicate ID |
| `422` | Unprocessable — valid request but business rule violation |
| `429` | Rate limited |
| `500` | Internal server error |

**Common error codes**

| Code | Description |
|---|---|
| `customer_not_found` | Customer ID does not exist |
| `plan_not_found` | Plan ID does not exist |
| `plan_archived` | Plan exists but is archived — cannot subscribe |
| `metric_not_found` | Metric referenced in event or charge does not exist |
| `duplicate_event` | Event with this `event_id` already exists — safely ignored |
| `duplicate_id` | Resource with this ID already exists |
| `active_subscriptions` | Cannot archive metric or plan with active subscriptions |
| `billing_run_exists` | A billing run for this customer+period already completed |
| `invalid_period` | Period start must be before period end |