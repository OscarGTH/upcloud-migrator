# ─────────────────────────────────────────────────────────────────────────────
# Databases — RDS PostgreSQL, ElastiCache Redis, parameter groups, subnet groups
# ─────────────────────────────────────────────────────────────────────────────

resource "aws_db_subnet_group" "main" {
  name       = "saas-db-subnet-group"
  subnet_ids = [aws_subnet.data.id]
  tags       = { Name = "saas-db-subnet-group" }
}

resource "aws_db_parameter_group" "postgres" {
  name   = "saas-postgres14-params"
  family = "postgres14"

  parameter {
    name  = "max_connections"
    value = "200"
  }

  parameter {
    name  = "shared_buffers"
    value = "131072"
  }

  parameter {
    name  = "log_min_duration_statement"
    value = "1000"
  }

  tags = { Name = "saas-postgres-params" }
}

resource "aws_db_instance" "postgres" {
  identifier        = "saas-postgres"
  engine            = "postgres"
  engine_version    = "14.10"
  instance_class    = var.db_instance_class
  allocated_storage = 100
  storage_type      = "gp3"

  db_name  = "saasdb"
  username = "saasadmin"
  password = "changeme-in-secrets-manager"

  db_subnet_group_name   = aws_db_subnet_group.main.name
  vpc_security_group_ids = [aws_security_group.postgres.id]
  parameter_group_name   = aws_db_parameter_group.postgres.name

  backup_retention_period = 7
  skip_final_snapshot     = false
  final_snapshot_identifier = "saas-postgres-final"

  tags = {
    Name        = "saas-postgres"
    Environment = var.environment
  }
}

# Redis cache
resource "aws_elasticache_subnet_group" "main" {
  name       = "saas-cache-subnet-group"
  subnet_ids = [aws_subnet.data.id]
  tags       = { Name = "saas-cache-subnet-group" }
}

resource "aws_elasticache_cluster" "redis" {
  cluster_id           = "saas-redis"
  engine               = "redis"
  node_type            = var.cache_node_type
  num_cache_nodes      = 1
  parameter_group_name = "default.redis7"
  engine_version       = "7.1"
  port                 = 6379

  subnet_group_name  = aws_elasticache_subnet_group.main.name
  security_group_ids = [aws_security_group.redis.id]

  tags = {
    Name        = "saas-redis"
    Environment = var.environment
  }
}
