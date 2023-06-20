https://github.com/nix-community/NixOS-WSL/releases/tag/22.05-5c211b47

https://github.com/nix-community/NixOS-WSL/releases/download/22.05-5c211b47/nixos-wsl-installer.tar.gz

https://github.com/nix-community/NixOS-WSL/releases/download/22.05-5c211b47/nixos-wsl-installer.tar.gz.sha256

```
PS C:\wsl> Get-History

  Id     Duration CommandLine
  --     -------- -----------
   1        6.829 wsl --import NixOS .\NixOS\ nixos-wsl-installer.tar.gz --version 2
   2       47.742 wsl -d NixOS
   3        1.986 wsl --shutdown
   4        0.046 wsl --list
   5       21.526 wsl -d NixOS
   6        0.053 wsl -s NixOS
   7        0.705 nix run github:Xe/gohello
   8     3:04.482 wsl
   9        0.729 wsl --shutdown
  10    26:18.340 wsl
```

```
[nixos@nixos:/mnt/c/wsl]$ history
    1  exit
    2  nix run github:Xe/gohello
    3  sudo nano /etc/nixos/configuration.nix
    4  nixos-rebuild
    5  curl http://test.local.developing.today
    6  exit
    7  curl http://test.local.developing.today
    8  sudo nano /etc/nixos/configuration.nix
    9  nixos-rebuild
   10  nixos-rebuild switch
   11  sudo nixos-rebuild switch
   12  curl localhost
   13  sudo mkdir -p /var/www/localhost
   14  chmod 777 -R /var/www
   15  sudo chmod 777 -R /var/www
   16  nano /var/www/index.html
   17  curl localhost
   18  curl localhost/index.html
   19  sudo chmod 777 -R /var/www
   20  curl localhost/index.html
   21  curl localhost
   22  sudo nano /etc/nixos/configuration.nix
   23  sudo nixos-rebuild
   24  sudo nixos-rebuild switch
   25  sudo nano /etc/nixos/configuration.nix
   26  sudo nixos-rebuild switch
   27  curl localhost
   28  curl localhost/
   29  curl localhost
   30  sudo nano /etc/nixos/configuration.nix
   31  sudo nixos-rebuild switch
   32  curl localhost
   33  curl localhost/index
   34  curl localhost/index.html
   35  ls -l /var/www
   36  chmod -R www-data:www-data /var/www
   37  chown www-data:www-data /var/www
   38  chuser
   39  chown nginx:nginx /var/www
   40  sudo chown nginx:nginx /var/www
   41  chmod 777 -R /var/www
   42  sudo chmod 777 -R /var/www
   43  sudo nixos-rebuild switch
   44  curl localhost
   45  sudo chmod 644 -R /var/www
   46  curl localhost
   47  sudo nixos-rebuild switch
   48  curl localhost
   49  ls /var/www/localhost
   50  ls /var/www
   51  sudo ls /var/www
   52  sudo ls -l
   53  sudo ls -l /var/www
   54  sudo ls -l /var/www/localhost
   55  mv /var/www/index /var/www/localhost
   56  sudo mv /var/www/index /var/www/localhost
   57  sudo mv /var/www/index.html /var/www/localhost
   58  sudo ls -l /var/www/localhost
   59  curl localhost
   60  curl localhost/index.html
   61  ls -l /var/www
   62  sudo ls -l /var/www
   63  chmod 777 -R /var/www
   64  sudo chmod 777 -R /var/www
   65  sudo chown nixos:users -R /var/www
   66  ls -lR /var/www
   67  curl localhost
   68  whoami
   69  ip addr
   70  curl localhost/index.html
   71  sudo nano /etc/nixos/configuration.nix
   72  sudo nixos-rebuild switch
   73  ip addr
   74  sudo nano /etc/nixos/configuration.nix
   75  ip addr
   76  sudo nixos-rebuild switch
   77  sudo nano /etc/nixos/configuration.nix
   78  ls /etc/nixos/configuration.nix
   79  ls /etc/nixos/
   80  exit
   81  sudo nano /etc/nixos/configuration.nix
   82  sudo nixos-rebuild switch
   83  curl localhost
   84  curl http://172.24.194.79/
   85  history
```

```
PowerShell 7.4.0-preview.3
PS C:\Users\drewr> curl http://172.24.194.79/
<h1>hello</h1>
PS C:\Users\drewr> Invoke-WebRequest http://172.24.194.79/

StatusCode        : 200
StatusDescription : OK
Content           : <h1>hello</h1>

RawContent        : HTTP/1.1 200 OK
                    Server: nginx
                    Date: Tue, 20 Jun 2023 05:24:26 GMT
                    Connection: keep-alive
                    ETag: "6491327c-f"
                    Accept-Ranges: bytes
                    Content-Type: text/html
                    Content-Length: 15
                    Last-Modified: Tue, …
Headers           : {[Server, System.String[]], [Date, System.String[]], [Connection, System.String[]], [ETag, System.String[]]…}
Images            : {}
InputFields       : {}
Links             : {}
RawContentLength  : 15
RelationLink      : {}

PS C:\Users\drewr>
```
