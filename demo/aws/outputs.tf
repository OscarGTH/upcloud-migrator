output "load_balancer_dns" {
  description = "Public DNS name of the load balancer"
  value       = aws_lb.main.dns_name
}

output "web_instance_ips" {
  description = "Public IP addresses of web servers"
  value       = aws_instance.web[*].public_ip
}

output "api_instance_private_ips" {
  description = "Private IP addresses of API servers"
  value       = aws_instance.api[*].private_ip
}

output "postgres_endpoint" {
  description = "PostgreSQL connection endpoint"
  value       = aws_db_instance.postgres.endpoint
}
