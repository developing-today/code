
add domain
https://monday.mxrouting.net:2222/evo/user/domains/add-domain

add forwarder
https://monday.mxrouting.net:2222/evo/user/email/forwarders
forward: developi@<domain> -> hi@developing-today.com

acquire dkim
https://monday.mxrouting.net:2222/evo/user/dns

add dns_config/default.nix
  (autosetup by adding domain)
  mx route 10 20
  txt
  (manual per-domain)
  dkim from above

enable ssl cert
(may need to double check mxrouting docs for this...)
https://mxroutedocs.com/branding/customhostnames/
https://monday.mxrouting.net:2222/evo/user/ssl/server
get automatic / use best match
webmail.<domain> mail.<domain>

set catchall email
https://monday.mxrouting.net:2222/evo/user/email/catch-all
address hi@developing-today.com
