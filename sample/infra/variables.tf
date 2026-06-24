variable "region" {
  description = "AWS region to deploy into"
  type        = string
  default     = "us-east-1"
}

variable "bucket_name" {
  description = "S3 bucket where datem stores Parquet data. Created if it does not exist."
  type        = string
}

variable "s3_prefix" {
  description = "Key prefix inside the bucket (e.g. 'datem')"
  type        = string
  default     = "datem"
}

variable "api_key" {
  description = "Bearer token datem uses to authenticate ingest HTTP calls (stored in SSM)"
  type        = string
  sensitive   = true
}

variable "stripe_key" {
  description = "Stripe secret key (sk_live_... or sk_test_...)"
  type        = string
  sensitive   = true
}

variable "stripe_webhook_secret" {
  description = "Stripe webhook signing secret (whsec_...)"
  type        = string
  sensitive   = true
}

variable "billing_cron" {
  description = "EventBridge cron expression for the billing run (UTC). Default: 1st of each month at 00:00."
  type        = string
  default     = "cron(0 0 1 * ? *)"
}

variable "ingest_batch_size" {
  description = "Number of SQS messages processed per Lambda invocation"
  type        = number
  default     = 100
}

variable "image_tag" {
  description = "Docker image tag to deploy (e.g. 'latest' or a git SHA)"
  type        = string
  default     = "latest"
}
