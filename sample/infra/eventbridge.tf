resource "aws_cloudwatch_event_rule" "billing" {
  name                = "datem-billing-cron"
  description         = "Trigger datem billing run on schedule"
  schedule_expression = var.billing_cron
}

resource "aws_cloudwatch_event_target" "billing" {
  rule      = aws_cloudwatch_event_rule.billing.name
  target_id = "datem-billing-lambda"
  arn       = aws_lambda_function.billing.arn
}

resource "aws_lambda_permission" "billing_eventbridge" {
  statement_id  = "AllowEventBridgeInvoke"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.billing.function_name
  principal     = "events.amazonaws.com"
  source_arn    = aws_cloudwatch_event_rule.billing.arn
}
