resource "aws_eks_cluster" "main" {
  name    = "myapp-cluster"
  version = "1.28"

  role_arn = aws_iam_role.eks_role.arn

  vpc_config {
    subnet_ids = [aws_subnet.private.id]
  }
}

resource "aws_eks_node_group" "workers" {
  cluster_name    = aws_eks_cluster.main.name
  node_group_name = "workers"
  node_role_arn   = aws_iam_role.node_role.arn

  scaling_config {
    desired_size = 2
    max_size     = 5
    min_size     = 1
  }

  instance_types = ["t3.medium"]
}

resource "aws_iam_role" "eks_role" {
  name = "eks-cluster-role"
  assume_role_policy = "{}"
}

resource "aws_iam_role" "node_role" {
  name = "eks-node-role"
  assume_role_policy = "{}"
}

resource "aws_lambda_function" "processor" {
  function_name = "data-processor"
  runtime       = "nodejs18.x"
  handler       = "index.handler"
  role          = aws_iam_role.eks_role.arn
  filename      = "function.zip"
}

resource "aws_sqs_queue" "tasks" {
  name = "task-queue"
}
