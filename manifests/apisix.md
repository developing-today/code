```bash
#https://github.com/myspotontheweb/gitops-workloads/blob/0d42b802ab98d65fc4628f5086bf42b11441e297/bin/setup.sh
#https://github.com/todaywasawesome/elite-cluster

helm repo add apisix https://charts.apiseven.com
helm repo update
helm pull apisix/apisix
helm pull apisix/apisix --untar
```
