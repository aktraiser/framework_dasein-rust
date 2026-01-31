# Firecracker Server

Serveur REST pour exécuter du code dans des microVMs Firecracker.

## Prérequis (sur le serveur Hetzner)

- Linux avec KVM (`/dev/kvm`)
- Firecracker installé (`/usr/local/bin/firecracker`)
- Kernel et rootfs (`/var/lib/firecracker/`)

## Installation

### 1. Sur votre Mac, compilez pour Linux :

```bash
# Installer la target Linux
rustup target add x86_64-unknown-linux-gnu

# Compiler (cross-compilation)
cd server/firecracker-server
cargo build --release --target x86_64-unknown-linux-gnu
```

Ou compilez directement sur le serveur :

### 2. Sur le serveur Hetzner :

```bash
# Installer Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Cloner/transférer le code
scp -r server/firecracker-server root@65.108.230.227:/root/

# Compiler
cd /root/firecracker-server
cargo build --release

# Installer
cp target/release/firecracker-server /usr/local/bin/
```

## Configuration

Variables d'environnement :

| Variable | Description | Défaut |
|----------|-------------|--------|
| `API_KEY` | Clé d'authentification | (aucune) |
| `PORT` | Port d'écoute | 8080 |

## Lancement

```bash
# Sans authentification (dev)
./firecracker-server

# Avec authentification (production)
API_KEY="votre-clé-secrète" ./firecracker-server

# En arrière-plan
API_KEY="votre-clé-secrète" nohup ./firecracker-server > /var/log/firecracker-server.log 2>&1 &
```

## Service systemd (recommandé)

Créez `/etc/systemd/system/firecracker-server.service` :

```ini
[Unit]
Description=Firecracker Sandbox Server
After=network.target

[Service]
Type=simple
User=root
Environment=API_KEY=votre-clé-secrète
Environment=PORT=8080
ExecStart=/usr/local/bin/firecracker-server
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Puis :

```bash
systemctl daemon-reload
systemctl enable firecracker-server
systemctl start firecracker-server
systemctl status firecracker-server
```

## API

### GET /health

Vérifier l'état du serveur.

```bash
curl http://65.108.230.227:8080/health
```

Réponse :
```json
{
  "status": "healthy",
  "firecracker_available": true,
  "kvm_available": true,
  "version": "0.1.0"
}
```

### POST /execute

Exécuter du code.

```bash
curl -X POST http://65.108.230.227:8080/execute \
  -H "Content-Type: application/json" \
  -H "X-API-Key: votre-clé-secrète" \
  -d '{"code": "echo Hello World", "timeout_ms": 5000}'
```

Réponse :
```json
{
  "success": true,
  "exit_code": 0,
  "stdout": "Hello World\n",
  "stderr": "",
  "execution_time_ms": 12
}
```

## Utilisation depuis le framework

```rust
use agentic_sandbox::{RemoteSandbox, Sandbox};

let sandbox = RemoteSandbox::new("http://65.108.230.227:8080")
    .api_key("votre-clé-secrète")
    .timeout_ms(30000)
    .build();

let result = sandbox.execute("echo 'Hello from Firecracker!'").await?;
println!("Output: {}", result.stdout);
```

## Sécurité

1. **Toujours utiliser une API_KEY en production**
2. **Configurer un firewall** pour n'autoriser que votre IP
3. **Utiliser HTTPS** avec un reverse proxy (nginx/caddy)

```bash
# Firewall : autoriser uniquement votre IP
ufw allow from VOTRE_IP to any port 8080
ufw enable
```
