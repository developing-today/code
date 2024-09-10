terraform {
  backend "local" {}
  required_providers {
    porkbun = {
      source = "cullenmcdermott/porkbun"
      # source  = "registry.terraform.io/developing-today-forks/porkbun"
      # version = "1.2.8"
    }
    sops = {
      source = "carlpett/sops"
    }
  }
}
data "sops_file" "porkbun" {
  source_file = "../../config.enc/common/porkbun.yaml"
}
provider "porkbun" {
  api_key = data.sops_file.porkbun.data["porkbun_api_key"]
  secret_key = data.sops_file.porkbun.data["porkbun_secret_key"]
  max_retries = 50
}
locals {
  dns_config = yamldecode(file("./dns_config.yaml"))["@"]
  porkbun_dns_records = local.dns_config["porkbun"]
}

# porkbun_dns_record
# # required
# type (String) The type of DNS Record to create
# domain (String) The base domain to to create the record on
# # optional
# name (String) The subdomain for the record itself without the base domain
# content (String) The content of the record
# notes (String) Notes to add to the record
# prio (String) The priority of the record
# ttl (String) The ttl of the record, the minimum is 600
resource "porkbun_dns_record" "record" {
  for_each = local.porkbun_dns_records
  type = lookup(each.value, "type", null)
  domain = lookup(each.value, "domain", null)
  name = lookup(each.value, "name", null)
  content = lookup(each.value, "content", null)
  notes = lookup(each.value, "notes", null)
  prio = lookup(each.value, "priority", null)
  ttl = lookup(each.value, "ttl", null)
}
