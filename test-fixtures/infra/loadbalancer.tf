resource "aws_lb" "web" {
  name               = "web-lb"
  internal           = false
  load_balancer_type = "application"
}

resource "aws_lb_target_group" "web" {
  name     = "web-tg"
  port     = 80
  protocol = "HTTP"
  vpc_id   = aws_vpc.main.id
}

resource "aws_lb_listener" "http" {
  load_balancer_arn = aws_lb.web.arn
  port              = 80
  protocol          = "HTTP"
}

resource "aws_acm_certificate" "ssl" {
  domain_name       = "example.com"
  validation_method = "DNS"
}

resource "aws_route53_record" "www" {
  zone_id = "ZONEID"
  name    = "www.example.com"
  type    = "A"
  ttl     = 300
  records = [aws_eip.nat.public_ip]
}
