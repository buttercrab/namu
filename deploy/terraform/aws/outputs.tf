output "cluster_name" {
  value = module.eks.cluster_name
}

output "kubeconfig_command" {
  value = "aws eks update-kubeconfig --region ${var.region} --name ${module.eks.cluster_name}"
}

output "redis_url_secret" {
  description = "The Secrets Manager secret holding REDIS_URL, for the `namu-redis` Secret."
  value       = aws_secretsmanager_secret.redis_url.name
}

output "artifacts_url" {
  description = "The chart's artifacts.url."
  value       = "s3://${aws_s3_bucket.artifacts.bucket}/tasks"
}

output "orchestrator_role_arn" {
  description = "The chart's orchestrator.serviceAccount.annotations[\"eks.amazonaws.com/role-arn\"]."
  value       = module.orchestrator_role.arn
}
