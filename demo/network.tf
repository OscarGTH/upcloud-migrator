# ─────────────────────────────────────────────────────────────────────────────
# Network — VPC, subnets, routing, security groups, elastic IPs
# ─────────────────────────────────────────────────────────────────────────────

resource "aws_vpc" "main" {
  cidr_block           = "10.0.0.0/16"
  enable_dns_support   = true
  enable_dns_hostnames = true

  tags = {
    Name        = "saas-vpc"
    Environment = var.environment
  }
}

# Public subnets (web tier + load balancer)
resource "aws_subnet" "public_a" {
  vpc_id            = aws_vpc.main.id
  cidr_block        = "10.0.10.0/24"
  availability_zone = "eu-west-1a"
  tags = { Name = "saas-public-a" }
}

resource "aws_subnet" "public_b" {
  vpc_id            = aws_vpc.main.id
  cidr_block        = "10.0.11.0/24"
  availability_zone = "eu-west-1b"
  tags = { Name = "saas-public-b" }
}

# Private subnets (API tier)
resource "aws_subnet" "private_a" {
  vpc_id            = aws_vpc.main.id
  cidr_block        = "10.0.12.0/24"
  availability_zone = "eu-west-1a"
  tags = { Name = "saas-private-a" }
}

resource "aws_subnet" "private_b" {
  vpc_id            = aws_vpc.main.id
  cidr_block        = "10.0.13.0/24"
  availability_zone = "eu-west-1b"
  tags = { Name = "saas-private-b" }
}

# Data subnet (databases + cache)
resource "aws_subnet" "data" {
  vpc_id            = aws_vpc.main.id
  cidr_block        = "10.0.20.0/24"
  availability_zone = "eu-west-1a"
  tags = { Name = "saas-data" }
}

resource "aws_internet_gateway" "igw" {
  vpc_id = aws_vpc.main.id
  tags   = { Name = "saas-igw" }
}

resource "aws_route_table" "public" {
  vpc_id = aws_vpc.main.id

  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.igw.id
  }

  tags = { Name = "saas-public-rt" }
}

resource "aws_route_table_association" "public_a" {
  subnet_id      = aws_subnet.public_a.id
  route_table_id = aws_route_table.public.id
}

resource "aws_route_table_association" "public_b" {
  subnet_id      = aws_subnet.public_b.id
  route_table_id = aws_route_table.public.id
}

# NAT Gateway for private subnet outbound
resource "aws_eip" "nat" {
  domain = "vpc"
  tags   = { Name = "saas-nat-eip" }
}

resource "aws_nat_gateway" "nat" {
  allocation_id = aws_eip.nat.id
  subnet_id     = aws_subnet.public_a.id
  tags          = { Name = "saas-nat" }
}

# ── Security Groups ────────────────────────────────────────────────────────────

# Load balancer: public HTTP + HTTPS
resource "aws_security_group" "lb" {
  name        = "saas-lb-sg"
  description = "Load balancer — allow HTTP and HTTPS from internet"
  vpc_id      = aws_vpc.main.id

  ingress {
    from_port   = 80
    to_port     = 80
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
    description = "HTTP"
  }

  ingress {
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
    description = "HTTPS"
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = { Name = "saas-lb-sg" }
}

# Web servers: traffic from LB + SSH
resource "aws_security_group" "web" {
  name        = "saas-web-sg"
  description = "Web servers — allow traffic from load balancer"
  vpc_id      = aws_vpc.main.id

  ingress {
    from_port       = 8080
    to_port         = 8080
    protocol        = "tcp"
    security_groups = [aws_security_group.lb.id]
    description     = "App port from LB"
  }

  ingress {
    from_port   = 22
    to_port     = 22
    protocol    = "tcp"
    cidr_blocks = ["10.0.0.0/16"]
    description = "SSH from VPC"
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = { Name = "saas-web-sg" }
}

# Monitoring: Prometheus node exporter on all servers
resource "aws_security_group" "monitoring" {
  name        = "saas-monitoring-sg"
  description = "Prometheus node exporter scraping"
  vpc_id      = aws_vpc.main.id

  ingress {
    from_port   = 9100
    to_port     = 9100
    protocol    = "tcp"
    cidr_blocks = ["10.0.0.0/16"]
    description = "Prometheus node exporter"
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = { Name = "saas-monitoring-sg" }
}

# API servers: from web tier only
resource "aws_security_group" "api" {
  name        = "saas-api-sg"
  description = "API servers — allow traffic from web tier"
  vpc_id      = aws_vpc.main.id

  ingress {
    from_port       = 3000
    to_port         = 3000
    protocol        = "tcp"
    security_groups = [aws_security_group.web.id]
    description     = "API from web tier"
  }

  ingress {
    from_port   = 22
    to_port     = 22
    protocol    = "tcp"
    cidr_blocks = ["10.0.0.0/16"]
    description = "SSH from VPC"
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = { Name = "saas-api-sg" }
}

# Postgres: from API tier only
resource "aws_security_group" "postgres" {
  name        = "saas-postgres-sg"
  description = "PostgreSQL — allow from API tier"
  vpc_id      = aws_vpc.main.id

  ingress {
    from_port       = 5432
    to_port         = 5432
    protocol        = "tcp"
    security_groups = [aws_security_group.api.id]
    description     = "PostgreSQL from API"
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = { Name = "saas-postgres-sg" }
}

# Redis: from API tier only
resource "aws_security_group" "redis" {
  name        = "saas-redis-sg"
  description = "Redis — allow from API tier"
  vpc_id      = aws_vpc.main.id

  ingress {
    from_port       = 6379
    to_port         = 6379
    protocol        = "tcp"
    security_groups = [aws_security_group.api.id]
    description     = "Redis from API"
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = { Name = "saas-redis-sg" }
}
