# ─────────────────────────────────────────────────────────────────────────────
# Load Balancer layer
# Public ALB + HTTPS listener + target groups + ACM certificate
# ─────────────────────────────────────────────────────────────────────────────

# TLS certificate  →  upcloud_loadbalancer_manual_certificate_bundle
# certificate and private_key attributes will remain as <TODO> — must be
# exported from ACM and base64-encoded manually.
resource "aws_acm_certificate" "webapp" {
  domain_name               = "webapp.example.com"
  subject_alternative_names = ["www.webapp.example.com"]
  validation_method         = "DNS"

  tags = { Name = "webapp-cert" }
}

# Public Application Load Balancer  →  upcloud_loadbalancer (networks { type = "public" })
resource "aws_lb" "web" {
  name               = "webapp-web-lb"
  internal           = false
  load_balancer_type = "application"

  subnets         = [aws_subnet.public_a.id, aws_subnet.public_b.id]
  security_groups = [aws_security_group.web.id]

  tags = { Name = "webapp-web-lb" }
}

# Internal ALB for the app tier  →  upcloud_loadbalancer (networks { type = "private" })
# The network reference will be auto-resolved if a subnet exists in the migration.
resource "aws_lb" "app_internal" {
  name               = "webapp-app-lb"
  internal           = true
  load_balancer_type = "application"

  subnets         = [aws_subnet.private_a.id, aws_subnet.private_b.id]
  security_groups = [aws_security_group.app.id]

  tags = { Name = "webapp-app-lb" }
}

# Web target group  →  upcloud_loadbalancer_backend + upcloud_loadbalancer_static_backend_member
resource "aws_lb_target_group" "web" {
  name     = "webapp-web-tg"
  port     = 80
  protocol = "HTTP"
  vpc_id   = aws_vpc.main.id

  health_check {
    path                = "/health"
    interval            = 30
    healthy_threshold   = 2
    unhealthy_threshold = 3
  }

  tags = { Name = "webapp-web-tg" }
}

# App target group (port 3000)
resource "aws_lb_target_group" "app" {
  name     = "webapp-app-tg"
  port     = 3000
  protocol = "HTTP"
  vpc_id   = aws_vpc.main.id

  health_check {
    path = "/api/health"
  }

  tags = { Name = "webapp-app-tg" }
}

# HTTP listener (port 80)  →  upcloud_loadbalancer_frontend
resource "aws_lb_listener" "http" {
  load_balancer_arn = aws_lb.web.arn
  port              = 80
  protocol          = "HTTP"

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.web.arn
  }
}

# HTTPS listener (port 443)  →  upcloud_loadbalancer_frontend
resource "aws_lb_listener" "https" {
  load_balancer_arn = aws_lb.web.arn
  port              = 443
  protocol          = "HTTPS"
  ssl_policy        = "ELBSecurityPolicy-TLS13-1-2-2021-06"
  certificate_arn   = aws_acm_certificate.webapp.arn

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.web.arn
  }
}

# Internal app listener  →  upcloud_loadbalancer_frontend
resource "aws_lb_listener" "app" {
  load_balancer_arn = aws_lb.app_internal.arn
  port              = 3000
  protocol          = "HTTP"

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.app.arn
  }
}
