terraform {
  required_providers {
    porkbun = {
      source = "cullenmcdermott/porkbun"
    }
    sops = {
      source = "carlpett/sops"
      version = "~> 0.5"
    }
  }
}
data "sops_file" "demo-secret" {
  source_file = "../../lib/config.enc/common/secrets.yaml"
}
provider "porkbun" {
  api_key = data.sops_file.demo-secret.data["porkbun_api_key"]
  secret_key = data.sops_file.demo-secret.data["porkbun_secret_key"]
}

# porkbun_dns_record
# # required
# type (String) The type of DNS Record to create
# domain (String) The base domain to to create the record on
# optional
# content (String) The content of the record
# name (String) The subdomain for the record itself without the base domain
# notes (String) Notes to add to the record
# prio (String) The priority of the record
# ttl (String) The ttl of the record, the minimum is 600

resource "porkbun_dns_record" "example" {
  type = "CNAME"
  domain = "64b.org"
  name = "www"
  content = "x.com"
  }
