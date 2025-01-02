#!/usr/bin/env bash

set -ex

. ./load-env.sh

argocd-autopilot repo bootstrap

# GITHUB_REPO="https://github.com/argoproj-labs/argocd-autopilot"
# ARGO_APP="hello-world"
# ARGO_APP_PATH="examples/demo-app/"
# ARGO_PROJECT="testing"
# . ./init-app.sh

# argocd-autopilot project create testing
# argocd-autopilot project create test1
# argocd-autopilot project create test
# argocd-autopilot project create infra-dev
# argocd-autopilot project create infra-development
# argocd-autopilot project create infra-prod
# argocd-autopilot project create infra-production
# argocd-autopilot project create dev
# argocd-autopilot project create development
# argocd-autopilot project create qa
# argocd-autopilot project create stage
# argocd-autopilot project create staging
# argocd-autopilot project create prod
# argocd-autopilot project create production

# argocd-autopilot project create testing-privileged

argocd-autopilot app create hello-world --app github.com/argoproj-labs/argocd-autopilot/examples/demo-app/ -p testing --wait-timeout 2m

argocd cluster add "admin@01b.us"

# maybe this should be different, a yaml file or something
kubectl patch svc argocd-server -n argocd -p '{"spec": {"type": "LoadBalancer"}}'

kubectl config set-context --current --namespace=argocd

kubectl get pods -n argocd -l app.kubernetes.io/name=argocd-server -o name | cut -d'/' -f 2

# kubectl port-forward -n argocd svc/argocd-server 8080:80
# kubectl port-forward -n argocd svc/argocd-server 9000:443
kubectl port-forward -n argocd svc/argocd-server 8080:443

# You can access Argo CD using port forwarding: add --port-forward-namespace argocd flag to every CLI command or set ARGOCD_OPTS environment variable: export ARGOCD_OPTS='--port-forward-namespace argocd':
# argocd login --port-forward
# argocd login --port-forward --port-forward-namespace argocd --plaintext 127.0.0.1:8080
argocd login localhost:8080 --insecure # login

argocd app create guestbook --repo https://github.com/argoproj/argocd-example-apps.git --path guestbook --dest-server https://kubernetes.default.svc --dest-namespace default

argocd app get guestbook

argocd app sync guestbook

kubectl expose deployment -n default guestbook-ui --type=LoadBalancer --port=80

# metallb hack to get addr without yaml changes, should be yaml
kubectl patch service guestbook-ui -p '{"spec": {"type": "LoadBalancer"}}' -n default
kubectl patch service simple-service -p '{"spec": {"type": "LoadBalancer"}}' -n default
# end metallb hack to avoid port-forward

kubectl get service -n default
# https://argo-cd.readthedocs.io/en/stable/user-guide/projects/
# PROJ=myproject
# APP=guestbook-default
# ROLE=get-role
# argocd proj role create $PROJ $ROLE
# argocd proj role create-token $PROJ $ROLE -e 10m
# JWT=<value from command above>
# argocd proj role list $PROJ
# argocd proj role get $PROJ $ROLE

# # This command will fail because the JWT Token associated with the project role does not have a policy to allow access to the application
# argocd app get $APP --auth-token $JWT
# # Adding a policy to grant access to the application for the new role
# argocd proj role add-policy $PROJ $ROLE --action get --permission allow --object $APP
# argocd app get $APP --auth-token $JWT

# # Removing the policy we added and adding one with a wildcard.
# argocd proj role remove-policy $PROJ $ROLE -a get -o $APP
# argocd proj role add-policy $PROJ $ROLE -a get --permission allow -o '*'
# # The wildcard allows us to access the application due to the wildcard.
# argocd app get $APP --auth-token $JWT
# argocd proj role get $PROJ $ROLE


# argocd proj role get $PROJ $ROLE
# # Revoking the JWT token
# argocd proj role delete-token $PROJ $ROLE <id field from the last command>
# # This will fail since the JWT Token was deleted for the project role.
# argocd app get $APP --auth-token $JWT
