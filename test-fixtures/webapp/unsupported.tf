# ─────────────────────────────────────────────────────────────────────────────
# Unsupported / partially-supported AWS services
# These appear in MIGRATION_NOTES.md — no UpCloud equivalent exists.
# Included to verify the tool correctly categorises them.
# ─────────────────────────────────────────────────────────────────────────────

# IAM  →  Unsupported
resource "aws_iam_role" "eks" {
  name = "webapp-eks-role"
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Action    = "sts:AssumeRole"
      Effect    = "Allow"
      Principal = { Service = "eks.amazonaws.com" }
    }]
  })
}

resource "aws_iam_role" "eks_node" {
  name = "webapp-eks-node-role"
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Action    = "sts:AssumeRole"
      Effect    = "Allow"
      Principal = { Service = "ec2.amazonaws.com" }
    }]
  })
}

resource "aws_iam_role_policy_attachment" "eks_policy" {
  policy_arn = "arn:aws:iam::aws:policy/AmazonEKSClusterPolicy"
  role       = aws_iam_role.eks.name
}

resource "aws_iam_role_policy_attachment" "eks_node_policy" {
  policy_arn = "arn:aws:iam::aws:policy/AmazonEKSWorkerNodePolicy"
  role       = aws_iam_role.eks_node.name
}

# Lambda  →  Unsupported (no serverless/FaaS)
resource "aws_lambda_function" "image_resize" {
  function_name = "webapp-image-resize"
  runtime       = "nodejs20.x"
  handler       = "index.handler"
  role          = aws_iam_role.eks.arn
  filename      = "lambda.zip"

  environment {
    variables = {
      BUCKET = aws_s3_bucket.uploads.id
    }
  }

  tags = { Name = "webapp-image-resize" }
}

resource "aws_lambda_permission" "s3_invoke" {
  statement_id  = "AllowS3Invoke"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.image_resize.function_name
  principal     = "s3.amazonaws.com"
}

# CloudFront CDN  →  Unsupported
resource "aws_cloudfront_distribution" "assets" {
  enabled = true

  origin {
    domain_name = aws_s3_bucket.assets.bucket_regional_domain_name
    origin_id   = "S3-webapp-assets"
  }

  default_cache_behavior {
    allowed_methods        = ["GET", "HEAD"]
    cached_methods         = ["GET", "HEAD"]
    target_origin_id       = "S3-webapp-assets"
    viewer_protocol_policy = "redirect-to-https"

    forwarded_values {
      query_string = false
      cookies { forward = "none" }
    }
  }

  restrictions {
    geo_restriction { restriction_type = "none" }
  }

  viewer_certificate {
    acm_certificate_arn = aws_acm_certificate.webapp.arn
    ssl_support_method  = "sni-only"
  }

  tags = { Name = "webapp-cdn" }
}

# SQS queue  →  Unsupported (no queue service)
resource "aws_sqs_queue" "jobs" {
  name                      = "webapp-jobs"
  delay_seconds             = 0
  max_message_size          = 262144
  message_retention_seconds = 86400
  visibility_timeout_seconds = 30

  tags = { Name = "webapp-jobs" }
}

resource "aws_sqs_queue" "jobs_dlq" {
  name = "webapp-jobs-dlq"
  tags = { Name = "webapp-jobs-dlq" }
}

# SNS  →  Unsupported (no pub/sub)
resource "aws_sns_topic" "alerts" {
  name = "webapp-alerts"
  tags = { Name = "webapp-alerts" }
}

resource "aws_sns_topic_subscription" "alerts_email" {
  topic_arn = aws_sns_topic.alerts.arn
  protocol  = "email"
  endpoint  = "ops@example.com"
}

# CloudWatch  →  Unsupported
resource "aws_cloudwatch_log_group" "app" {
  name              = "/webapp/app"
  retention_in_days = 30
  tags              = { Name = "webapp-app-logs" }
}

resource "aws_cloudwatch_metric_alarm" "cpu_high" {
  alarm_name          = "webapp-cpu-high"
  comparison_operator = "GreaterThanThreshold"
  evaluation_periods  = 2
  metric_name         = "CPUUtilization"
  namespace           = "AWS/EC2"
  period              = 120
  statistic           = "Average"
  threshold           = 80
  alarm_description   = "CPU above 80%"
}

# Route53 DNS  →  Partial (must be managed outside Terraform or via DNS provider)
resource "aws_route53_zone" "main" {
  name = "webapp.example.com"
  tags = { Name = "webapp-zone" }
}

resource "aws_route53_record" "www" {
  zone_id = aws_route53_zone.main.zone_id
  name    = "www.webapp.example.com"
  type    = "A"

  alias {
    name                   = aws_lb.web.dns_name
    zone_id                = aws_lb.web.zone_id
    evaluate_target_health = true
  }
}

resource "aws_route53_record" "api" {
  zone_id = aws_route53_zone.main.zone_id
  name    = "api.webapp.example.com"
  type    = "A"
  ttl     = 300
  records = [aws_eip.lb_static.public_ip]
}
