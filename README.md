# upcloud-migrate

A terminal tool that converts AWS Terraform infrastructure into UpCloud Terraform — automatically, in your terminal.

Point it at an existing AWS Terraform project and it maps every resource, resolves cross-references, generates valid UpCloud HCL, and highlights everything that needs manual review.

![TUI screenshot placeholder]

---

## What it does

- **Scans** your `.tf` files and identifies every AWS resource
- **Maps** each resource to its closest UpCloud equivalent with generated HCL
- **Resolves** cross-references automatically: security group → server, load balancer → backend, subnet → network
- **Merges** multiple security groups attached to the same server into a single `upcloud_firewall_rules` resource (UpCloud requires exactly one per server)
- **Rewrites** variables, outputs, and locals — instance type defaults are converted to UpCloud plan strings, AWS region references become zone placeholders
- **Flags** everything it can't fully automate with `<TODO>` markers and a post-generation review screen powered by AI suggestions
- **Diffs** every resource side-by-side: original AWS HCL on the left, generated UpCloud HCL on the right

---

## Supported resources

| AWS resource | UpCloud equivalent |
|---|---|
| `aws_vpc` | `upcloud_router` |
| `aws_subnet` | `upcloud_network` |
| `aws_security_group` | `upcloud_firewall_rules` |
| `aws_internet_gateway` | *(informational note — not needed)* |
| `aws_nat_gateway` | *(informational note — not needed)* |
| `aws_route_table` | `static_route` snippet for `upcloud_router` |
| `aws_route_table_association` | *(informational note — handled with route_table)* |
| `aws_eip` | `upcloud_floating_ip_address` |
| `aws_eip_association` | `mac_address` snippet for floating IP |
| `aws_instance` | `upcloud_server` |
| `aws_key_pair` | `login {}` block in `upcloud_server` |
| `aws_ebs_volume` | `upcloud_storage` |
| `aws_volume_attachment` | `storage_devices {}` snippet for server |
| `aws_s3_bucket` | `upcloud_object_storage` |
| `aws_efs_file_system` | `upcloud_file_storage` |
| `aws_lb` / `aws_alb` | `upcloud_loadbalancer` |
| `aws_lb_target_group` | `upcloud_loadbalancer_backend` |
| `aws_lb_listener` | `upcloud_loadbalancer_frontend` |
| `aws_lb_target_group_attachment` | `upcloud_loadbalancer_static_backend_member` |
| `aws_acm_certificate` | `upcloud_loadbalancer_manual_certificate_bundle` |
| `aws_db_instance` | `upcloud_managed_database_postgresql` / `_mysql` |
| `aws_rds_cluster` | `upcloud_managed_database_postgresql` / `_mysql` |
| `aws_db_parameter_group` | `properties {}` block injection |
| `aws_db_subnet_group` | *(informational note — network configured in managed DB)* |
| `aws_elasticache_cluster` | `upcloud_managed_database_valkey` |
| `variable` / `output` / `locals` | passed through with AWS refs rewritten |

---

## Usage

```bash
cargo build --release
./target/release/upcloud-migrate
```

The TUI walks you through four steps:

1. **Path** — enter the directory containing your `.tf` files
2. **Scan** — the tool discovers and parses all Terraform files
3. **Resources** — browse every mapped resource with a live HCL preview
4. **Generate** — pick a zone and output directory; the tool writes ready-to-review `.tf` files

After generation:

- **`[D]` Diff** — step through every resource with AWS HCL on the left and generated UpCloud HCL on the right
- **`[T]` TODOs** — review every unresolved `<TODO>` with AI-suggested completions (requires env. vars `LLM_API_KEY`, `LLM_API_URL`, `LLM_MODEL`)

### Try it with the demo fixture

```bash
# Run the tool and enter this path when prompted:
demo/
```

The `demo/` directory contains a realistic SaaS app: web + API servers, PostgreSQL, Redis, an Application Load Balancer with TLS termination, S3 buckets, and a shared EFS filesystem — spread across five `.tf` files to show multi-file handling.

---

## Key behaviours

**Security group merging** — AWS allows multiple security groups per instance. UpCloud allows exactly one `upcloud_firewall_rules` per server. When multiple SGs are attached to the same instance via `vpc_security_group_ids`, the generator merges all rules into a single resource and deduplicates catch-all entries.

**Cross-reference resolution** — `server_id`, `network`, `loadbalancer`, and `backend` references are resolved automatically where the mapping is unambiguous. Ambiguous references (multiple servers, multiple networks) get a `<TODO>` with context.

**Variable passthrough** — `variable`, `output`, and `locals` blocks are carried through to the output. Instance type variable defaults (e.g. `"t3.medium"`) are converted to the equivalent UpCloud plan string. Region variables are replaced with the zone you selected.

**Root volume size** — `root_block_device.volume_size` is read and propagated to the `upcloud_server` template block.

**HCL validation** — each generated file is validated with `hcl::from_str` after writing. Invalid files are flagged in the generation log.

---

## Environment variables

| Variable | Purpose |
|---|---|
| `LLM_API_KEY` | Enables AI-powered TODO suggestions in the review screen |
| `LLM_API_URL` | Enables AI-powered TODO suggestions in the review screen |
| `LLM_MODEL` | Enables AI-powered TODO suggestions in the review screen |

---

## Building

Requires Rust 1.75+ (edition 2024).

```bash
cargo build --release
```

Dependencies: `ratatui`, `crossterm`, `tokio`, `hcl-rs`, `walkdir`, `reqwest`.

---

## What it won't do

The generated output is a **starting point**, not a drop-in replacement. Things that always require manual review:

- OS template selection (`"Ubuntu Server 24.04 LTS"` is the default — change it to match your AMI)
- SSH keys for servers without `aws_key_pair` references
- ACM certificates (AWS doesn't expose private keys — export from ACM manually)
- EKS clusters (no direct UpCloud Kubernetes equivalent in this tool's scope)
- Resources with no UpCloud equivalent (autoscaling groups, IAM, CloudWatch) are documented in `MIGRATION_NOTES.md`
