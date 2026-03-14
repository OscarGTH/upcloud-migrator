# ─────────────────────────────────────────────────────────────────────────────
# Database layer
# RDS PostgreSQL, Aurora MySQL cluster, ElastiCache Redis
# ─────────────────────────────────────────────────────────────────────────────

# PostgreSQL RDS  →  upcloud_managed_database_postgresql
resource "aws_db_instance" "postgres" {
  identifier        = "webapp-postgres"
  engine            = "postgres"
  engine_version    = "15.4"
  instance_class    = "db.t3.medium"
  allocated_storage = 100
  storage_type      = "gp3"

  db_name  = "webapp"
  username = "webapp_admin"
  password = "changeme-use-secrets-manager"

  db_subnet_group_name   = "webapp-db-subnet-group"
  vpc_security_group_ids = [aws_security_group.db.id]

  backup_retention_period = 7
  skip_final_snapshot     = false
  final_snapshot_identifier = "webapp-postgres-final"

  tags = {
    Name    = "webapp-postgres"
    Project = "webapp"
  }
}

# MySQL RDS  →  upcloud_managed_database_mysql
resource "aws_db_instance" "mysql_reports" {
  identifier        = "webapp-mysql-reports"
  engine            = "mysql"
  engine_version    = "8.0"
  instance_class    = "db.t3.large"
  allocated_storage = 200
  storage_type      = "gp3"

  db_name  = "reports"
  username = "reports_admin"
  password = "changeme-use-secrets-manager"

  db_subnet_group_name   = "webapp-db-subnet-group"
  vpc_security_group_ids = [aws_security_group.db.id]

  tags = {
    Name    = "webapp-mysql-reports"
    Project = "webapp"
  }
}

# Aurora PostgreSQL cluster  →  upcloud_managed_database_postgresql
resource "aws_rds_cluster" "aurora" {
  cluster_identifier = "webapp-aurora"
  engine             = "aurora-postgresql"
  engine_version     = "15.4"
  database_name      = "webapp_cluster"
  master_username    = "cluster_admin"
  master_password    = "changeme-use-secrets-manager"

  db_subnet_group_name   = "webapp-db-subnet-group"
  vpc_security_group_ids = [aws_security_group.db.id]

  backup_retention_period = 7
  preferred_backup_window = "02:00-03:00"
  skip_final_snapshot     = false

  tags = {
    Name    = "webapp-aurora"
    Project = "webapp"
  }
}

# Redis ElastiCache  →  upcloud_managed_database_valkey
resource "aws_elasticache_cluster" "session" {
  cluster_id           = "webapp-session"
  engine               = "redis"
  node_type            = "cache.t3.micro"
  num_cache_nodes      = 1
  parameter_group_name = "default.redis7"
  engine_version       = "7.0"
  port                 = 6379

  subnet_group_name  = "webapp-cache-subnet-group"
  security_group_ids = [aws_security_group.cache.id]

  tags = {
    Name    = "webapp-session-cache"
    Project = "webapp"
  }
}

# Redis ElastiCache replication group  →  upcloud_managed_database_valkey
resource "aws_elasticache_replication_group" "queue" {
  replication_group_id = "webapp-queue"
  description          = "Queue processing cache"
  engine               = "redis"
  node_type            = "cache.t3.small"
  num_cache_clusters   = 2
  engine_version       = "7.0"
  port                 = 6379

  subnet_group_name  = "webapp-cache-subnet-group"
  security_group_ids = [aws_security_group.cache.id]

  tags = {
    Name    = "webapp-queue-cache"
    Project = "webapp"
  }
}
