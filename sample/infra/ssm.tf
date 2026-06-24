resource "aws_ssm_parameter" "api_key" {
  name  = "/datem/api_key"
  type  = "SecureString"
  value = var.api_key
}

resource "aws_ssm_parameter" "stripe_key" {
  name  = "/datem/stripe_key"
  type  = "SecureString"
  value = var.stripe_key
}

resource "aws_ssm_parameter" "stripe_webhook_secret" {
  name  = "/datem/stripe_webhook_secret"
  type  = "SecureString"
  value = var.stripe_webhook_secret
}
