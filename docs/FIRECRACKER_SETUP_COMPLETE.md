# Guide Complet Firecracker - agentic-rs

## Architecture

```
┌─────────────────────────────┐                    ┌─────────────────────────────┐
│   MAC (Développement)       │                    │   HETZNER (Production)      │
│                             │                    │                             │
│   - VS Code                 │     HTTP API       │   - Ubuntu 22.04            │
│   - Framework agentic-rs    │ ────────────────►  │   - KVM activé              │
│   - RemoteSandbox           │                    │   - Firecracker             │
│                             │ ◄────────────────  │   - MicroVM                 │
│                             │     Résultat       │                             │
└─────────────────────────────┘                    └─────────────────────────────┘

                                                   ┌─────────────────────────────┐
                                                   │   MicroVM (172.16.0.2)      │
                                                   │                             │
                                                   │   - Python 3                │
                                                   │   - Node.js 20              │
                                                   │   - Go 1.21.5               │
                                                   │   - Rust 1.93.0             │
                                                   │   - fc-agent (port 8080)    │
                                                   └─────────────────────────────┘
```

## Informations Serveur

| Paramètre | Valeur |
|-----------|--------|
| **Serveur** | Hetzner AX41 (dédié) |
| **IPv4** | 65.108.230.227 |
| **OS** | Ubuntu 22.04 LTS |
| **KVM** | `/dev/kvm` disponible |

## Fichiers sur le Serveur Hetzner

| Fichier | Chemin | Description |
|---------|--------|-------------|
| Firecracker | `/usr/local/bin/firecracker` | Binaire Firecracker v1.6.0 |
| Kernel | `/var/lib/firecracker/vmlinux` | Kernel Linux 5.10.198 |
| Rootfs | `/var/lib/firecracker/rootfs-agent.ext4` | Ubuntu 22.04 rootfs (4GB) |
| VM Config | `/var/lib/firecracker/vm-config.json` | Configuration de la VM |

---

## Configuration de la VM

### `/var/lib/firecracker/vm-config.json`

```json
{
  "boot-source": {
    "kernel_image_path": "/var/lib/firecracker/vmlinux",
    "boot_args": "console=ttyS0 reboot=k panic=1 pci=off init=/sbin/init"
  },
  "drives": [
    {
      "drive_id": "rootfs",
      "path_on_host": "/var/lib/firecracker/rootfs-agent.ext4",
      "is_root_device": true,
      "is_read_only": false
    }
  ],
  "network-interfaces": [
    {
      "iface_id": "eth0",
      "guest_mac": "AA:FC:00:00:00:01",
      "host_dev_name": "tap0"
    }
  ],
  "machine-config": {
    "vcpu_count": 2,
    "mem_size_mib": 512
  }
}
```

---

## Configuration Réseau

### Réseau TAP (Hôte)

| Paramètre | Valeur |
|-----------|--------|
| Interface | `tap0` |
| IP Hôte | `172.16.0.1/24` |
| IP VM | `172.16.0.2/24` |

### Commandes pour créer le TAP

```bash
ip tuntap add dev tap0 mode tap
ip addr add 172.16.0.1/24 dev tap0
ip link set tap0 up
```

### Commandes pour recréer le TAP (si erreur)

```bash
ip link delete tap0
ip tuntap add dev tap0 mode tap
ip addr add 172.16.0.1/24 dev tap0
ip link set tap0 up
```

---

## Services dans la VM

### Service fcnet (Configuration Réseau)

**Fichier :** `/etc/systemd/system/fcnet.service`

```ini
[Unit]
Description=Configure Firecracker Network
Before=network.target
Wants=network.target

[Service]
Type=oneshot
ExecStart=/sbin/ip link set eth0 up
ExecStart=/sbin/ip addr add 172.16.0.2/24 dev eth0
ExecStart=/sbin/ip route add default via 172.16.0.1
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
```

### Service fc-agent (Agent d'Exécution)

**Fichier :** `/etc/systemd/system/fc-agent.service`

```ini
[Unit]
Description=Firecracker Agent
After=network.target

[Service]
Type=simple
ExecStart=/usr/bin/python3 /opt/fc-agent/agent.py
Restart=always
RestartSec=2
WorkingDirectory=/opt/fc-agent

[Install]
WantedBy=multi-user.target
```

---

## Agent Python

### `/opt/fc-agent/agent.py`

```python
#!/usr/bin/env python3
import http.server
import json
import subprocess
import socketserver

PORT = 8080

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == '/health':
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps({"status": "healthy"}).encode())

    def do_POST(self):
        if self.path == '/execute':
            length = int(self.headers.get('Content-Length', 0))
            body = json.loads(self.rfile.read(length))
            code = body.get('code', '')
            timeout = body.get('timeout_ms', 5000) / 1000
            try:
                result = subprocess.run(['sh', '-c', code], capture_output=True, text=True, timeout=timeout)
                response = {"success": True, "exit_code": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
            except subprocess.TimeoutExpired:
                response = {"success": False, "error": "timeout"}
            except Exception as e:
                response = {"success": False, "error": str(e)}
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps(response).encode())

with socketserver.TCPServer(("0.0.0.0", PORT), Handler) as httpd:
    print(f"Agent listening on port {PORT}")
    httpd.serve_forever()
```

---

## Commandes de Démarrage

### Démarrage Complet (depuis zéro)

```bash
# 1. Créer l'interface TAP
ip tuntap add dev tap0 mode tap
ip addr add 172.16.0.1/24 dev tap0
ip link set tap0 up

# 2. Lancer Firecracker
rm -f /tmp/firecracker.socket
firecracker --api-sock /tmp/firecracker.socket --config-file /var/lib/firecracker/vm-config.json
```

### Redémarrage (après arrêt)

```bash
# 1. Recréer le TAP
ip link delete tap0
ip tuntap add dev tap0 mode tap
ip addr add 172.16.0.1/24 dev tap0
ip link set tap0 up

# 2. Lancer Firecracker
rm -f /tmp/firecracker.socket
firecracker --api-sock /tmp/firecracker.socket --config-file /var/lib/firecracker/vm-config.json
```

### Arrêt

```bash
# Ctrl+C dans le terminal Firecracker
# Ou depuis un autre terminal :
pkill firecracker
```

---

## Tests de l'API

### Health Check

```bash
curl http://172.16.0.2:8080/health
```

**Réponse :**
```json
{"status": "healthy"}
```

### Exécution de Code

```bash
# Shell
curl -X POST http://172.16.0.2:8080/execute -H "Content-Type: application/json" -d '{"code":"ls /","timeout_ms":5000}'

# Python
curl -X POST http://172.16.0.2:8080/execute -H "Content-Type: application/json" -d '{"code":"python3 -c \"print(2+2)\"","timeout_ms":5000}'

# Node.js
curl -X POST http://172.16.0.2:8080/execute -H "Content-Type: application/json" -d '{"code":"node -e \"console.log(2+2)\"","timeout_ms":5000}'

# Go
curl -X POST http://172.16.0.2:8080/execute -H "Content-Type: application/json" -d '{"code":"go version","timeout_ms":5000}'

# Rust
curl -X POST http://172.16.0.2:8080/execute -H "Content-Type: application/json" -d '{"code":"rustc --version","timeout_ms":5000}'
```

**Réponse type :**
```json
{"success": true, "exit_code": 0, "stdout": "go version go1.21.5 linux/amd64\n", "stderr": ""}
```

---

## Modification du Rootfs

### Monter le Rootfs

```bash
# Arrêter Firecracker d'abord (Ctrl+C)
mkdir -p /mnt/rootfs
mount /var/lib/firecracker/rootfs-agent.ext4 /mnt/rootfs
```

### Éditer des fichiers

```bash
nano /mnt/rootfs/chemin/vers/fichier
```

### Démonter

```bash
umount /mnt/rootfs
```

### Activer un service systemd

```bash
ln -sf /etc/systemd/system/MON_SERVICE.service /mnt/rootfs/etc/systemd/system/multi-user.target.wants/MON_SERVICE.service
```

---

## Langages Installés dans la VM

| Langage | Version | Commande de test |
|---------|---------|------------------|
| Python | 3.10 | `python3 --version` |
| Node.js | v20.20.0 | `node --version` |
| Go | 1.21.5 | `go version` |
| Rust | 1.93.0 | `rustc --version` |

---

## Dépannage

### "Resource busy" sur tap0

```bash
ip link delete tap0
ip tuntap add dev tap0 mode tap
ip addr add 172.16.0.1/24 dev tap0
ip link set tap0 up
```

### "No route to host"

Vérifiez dans la VM :
```bash
ip addr show eth0
ip route
```

Si pas d'IP, configurez manuellement :
```bash
ip link set eth0 up
ip addr add 172.16.0.2/24 dev eth0
ip route add default via 172.16.0.1
```

### "Address already in use" (port 8080)

```bash
pkill -f agent.py
systemctl start fc-agent
```

### Service fc-agent ne démarre pas

```bash
journalctl -u fc-agent -n 50
```

### "File descriptor in bad state"

Le TAP est corrompu. Redémarrez tout :
```bash
# Ctrl+C sur Firecracker
ip link delete tap0
ip tuntap add dev tap0 mode tap
ip addr add 172.16.0.1/24 dev tap0
ip link set tap0 up
rm -f /tmp/firecracker.socket
firecracker --api-sock /tmp/firecracker.socket --config-file /var/lib/firecracker/vm-config.json
```

---

## Utilisation depuis le Mac (RemoteSandbox)

### Code Rust

```rust
use dasein_agentic_sandbox::{RemoteSandbox, Sandbox};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sandbox = RemoteSandbox::new("http://65.108.230.227:8080")
        .api_key("votre-clé-secrète")
        .timeout_ms(30000)
        .build();

    if sandbox.is_ready().await? {
        println!("Serveur Firecracker prêt !");
    }

    let result = sandbox.execute("echo 'Hello from Firecracker!'").await?;
    println!("Exit code: {}", result.exit_code);
    println!("Stdout: {}", result.stdout);

    Ok(())
}
```

### Activer la feature remote

```bash
cargo build --features remote
```

---

## Ports

| Port | Service | Accès |
|------|---------|-------|
| 22 | SSH | Depuis votre IP |
| 8080 | fc-agent (dans VM) | Interne (172.16.0.2) |

---

## Sécurité (Production)

### Configurer le firewall

```bash
ufw allow from VOTRE_IP to any port 22
ufw allow from VOTRE_IP to any port 8080
ufw enable
```

### Utiliser une API Key

L'agent actuel n'a pas d'authentification. Pour la production, ajoutez une vérification d'API key dans `agent.py`.

### HTTPS avec Caddy

```bash
apt install -y caddy
```

`/etc/caddy/Caddyfile`:
```
firecracker.votre-domaine.com {
    reverse_proxy 172.16.0.2:8080
}
```

```bash
systemctl restart caddy
```

  3. Démontez et relancez :                                                                                                 
  umount /mnt/rootfs                                                                                                        
  ip link delete tap0                                                                                                       
  ip tuntap add dev tap0 mode tap                                                                                           
  ip addr add 172.16.0.1/24 dev tap0                                                                                        
  ip link set tap0 up                                                                                                       
  rm -f /tmp/firecracker.socket                                                                                             
  firecracker --api-sock /tmp/firecracker.socket --config-file /var/lib/firecracker/vm-config.json   

   curl http://172.16.0.2:8080/health    

     Sur le serveur Hetzner, configurez le port forwarding :                                                                   
                                                                                                                            
  # Activer le forwarding IP                                                                                                
  echo 1 > /proc/sys/net/ipv4/ip_forward                                                                                    
                                                                                                                            
  # Rediriger le port 8080 public vers la VM                                                                                
  iptables -t nat -A PREROUTING -p tcp --dport 8080 -j DNAT --to-destination 172.16.0.2:8080                                
  iptables -t nat -A POSTROUTING -p tcp -d 172.16.0.2 --dport 8080 -j MASQUERADE                                            
                                                                                                                            
  Testez depuis votre Mac :                                                                                                 
                                                                                                                            
  curl http://65.108.230.227:8080/health      

  ssh root@65.108.230.227