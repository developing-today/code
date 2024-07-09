# shellcheck disable=SC2312
sudo NIX_CONFIG="access-tokens = github.com=$(cat /home/user/auth)" ./rebuild.sh
