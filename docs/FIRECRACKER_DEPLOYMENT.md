# Guide de Déploiement Firecracker

Ce guide documente l'installation et le déploiement complet de Firecracker sur un serveur dédié Hetzner pour le framework agentic-rs.

## Architecture

```
┌─────────────────────────────┐                    ┌─────────────────────────────┐
│   MAC (Développement)       │                    │   HETZNER (Production)      │
│                             │                    │                             │
│   - VS Code                 │     HTTP API       │   - Ubuntu 22.04            │
│   - Framework agentic-rs    │ ────────────────►  │   - KVM activé              │
│   - RemoteSandbox           │                    │   - Firecracker             │
│                             │ ◄────────────────  │   - firecracker-server      │
│                             │     Résultat       │                             │
└─────────────────────────────┘                    └─────────────────────────────┘
```

## Prérequis

### Serveur requis

| Critère | Requis | Notre serveur |
|---------|--------|---------------|
| **OS** | Linux (Ubuntu 22.04 recommandé) | ✅ Ubuntu 22.04 |
| **KVM** | `/dev/kvm` disponible | ✅ Disponible |
| **Type** | Serveur dédié (bare metal) | ✅ Hetzner AX41 |
| **RAM** | Minimum 4GB | ✅ 64GB |

> ⚠️ **Important** : Les VPS classiques (OVH, DigitalOcean, etc.) ne supportent généralement PAS KVM. Il faut un serveur **dédié/bare metal**.

### Notre serveur Hetzner

```
IPv4: 65.108.230.227
OS: Ubuntu 22.04 LTS
Type: Serveur dédié AX41
```

---

## Étape 1 : Connexion au serveur

```bash
ssh root@65.108.230.227
```

---

## Étape 2 : Vérification KVM

```bash
ls -la /dev/kvm
```

**Résultat attendu :**
```
crw-rw---- 1 root kvm 10, 232 Jan 27 08:44 /dev/kvm
```

Si `/dev/kvm` n'existe pas, le serveur ne supporte pas KVM (changer de serveur).

---

## Étape 3 : Installation des dépendances

```bash
apt update && apt install -y wget curl jq build-essential pkg-config libssl-dev
```

---

## Étape 4 : Installation de Firecracker

### Téléchargement

```bash
wget https://github.com/firecracker-microvm/firecracker/releases/download/v1.6.0/firecracker-v1.6.0-x86_64.tgz
```

### Extraction et installation

```bash
tar xzf firecracker-v1.6.0-x86_64.tgz
```

```bash
mv release-v1.6.0-x86_64/firecracker-v1.6.0-x86_64 /usr/local/bin/firecracker
```

```bash
chmod +x /usr/local/bin/firecracker
```

### Vérification

```bash
firecracker --version
```

**Résultat attendu :**
```
Firecracker v1.6.0
```

---

## Étape 5 : Téléchargement du Kernel et Rootfs

### Création du répertoire

```bash
mkdir -p /var/lib/firecracker && cd /var/lib/firecracker
```

### Téléchargement du kernel Linux

```bash
wget -O vmlinux https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.6/x86_64/vmlinux-5.10.198
```

### Téléchargement du rootfs Ubuntu

```bash
wget -O rootfs.ext4 https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.6/x86_64/ubuntu-22.04.ext4
```

### Vérification

```bash
ls -lh /var/lib/firecracker/
```

**Résultat attendu :**
```
total 327M
-rw-r--r-- 1 root root 300M ... rootfs.ext4
-rw-r--r-- 1 root root  27M ... vmlinux
```

---

## Étape 6 : Test de Firecracker

```bash
firecracker --api-sock /tmp/firecracker.socket
```

Attendez 2-3 secondes, puis `Ctrl+C` pour arrêter.

**Résultat attendu :**
```
Running Firecracker v1.6.0
```

---

## Étape 7 : Installation de Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Tapez `1` pour l'installation par défaut.

```bash
source ~/.cargo/env
```

### Vérification

```bash
rustc --version
cargo --version
```

---

## Étape 8 : Déploiement du firecracker-server

### Depuis le Mac, transférer le code

```bash
scp -r /Users/rbometon/Downloads/agentic-rs/server root@65.108.230.227:/root/
```

### Sur le serveur, compiler

```bash
cd /root/server/firecracker-server
cargo build --release
```

### Installation du binaire (optionnel)

```bash
cp target/release/firecracker-server /usr/local/bin/
```

---

## Étape 9 : Lancement du serveur

### Lancement manuel (test)

```bash
API_KEY="votre-clé-secrète" ./target/release/firecracker-server
```

### Lancement en arrière-plan

```bash
API_KEY="votre-clé-secrète" nohup ./target/release/firecracker-server > /var/log/firecracker-server.log 2>&1 &
```

### Configuration systemd (recommandé pour production)

Créer `/etc/systemd/system/firecracker-server.service` :

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
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

Activer et démarrer :

```bash
systemctl daemon-reload
systemctl enable firecracker-server
systemctl start firecracker-server
systemctl status firecracker-server
```

---

## Étape 10 : Test depuis le Mac

### Health check

```bash
curl http://65.108.230.227:8080/health
```

**Résultat attendu :**
```json
{
  "status": "healthy",
  "firecracker_available": true,
  "kvm_available": true,
  "version": "0.1.0"
}
```

### Test d'exécution

```bash
curl -X POST http://65.108.230.227:8080/execute \
  -H "Content-Type: application/json" \
  -H "X-API-Key: votre-clé-secrète" \
  -d '{"code": "echo Hello World", "timeout_ms": 5000}'
```

**Résultat attendu :**
```json
{
  "success": true,
  "exit_code": 0,
  "stdout": "Hello World\n",
  "stderr": "",
  "execution_time_ms": 12
}
```

---

## Utilisation dans le Framework

### Activer la feature remote

```bash
cargo build --features remote
```

### Code Rust

```rust
use dasein_agentic_sandbox::{RemoteSandbox, Sandbox};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Créer le sandbox distant
    let sandbox = RemoteSandbox::new("http://65.108.230.227:8080")
        .api_key("votre-clé-secrète")
        .timeout_ms(30000)
        .build();

    // Vérifier que le serveur est prêt
    if sandbox.is_ready().await? {
        println!("Serveur Firecracker prêt !");
    }

    // Exécuter du code
    let result = sandbox.execute("echo 'Hello from Firecracker!'").await?;

    println!("Exit code: {}", result.exit_code);
    println!("Stdout: {}", result.stdout);
    println!("Stderr: {}", result.stderr);
    println!("Temps: {}ms", result.execution_time_ms);

    Ok(())
}
```

---

## Sécurité

### 1. Toujours utiliser une API_KEY

```bash
API_KEY="une-clé-longue-et-complexe" ./firecracker-server
```

### 2. Configurer le firewall

```bash
# Autoriser uniquement votre IP
ufw allow from VOTRE_IP_MAC to any port 8080
ufw allow ssh
ufw enable
```

### 3. Utiliser HTTPS (production)

Installer Caddy comme reverse proxy :

```bash
apt install -y caddy
```

Créer `/etc/caddy/Caddyfile` :

```
firecracker.votre-domaine.com {
    reverse_proxy localhost:8080
}
```

```bash
systemctl restart caddy
```

---

## Dépannage

### Le serveur ne démarre pas

```bash
# Vérifier les logs
journalctl -u firecracker-server -f

# Ou si lancé manuellement
cat /var/log/firecracker-server.log
```

### KVM non disponible

```bash
# Vérifier
ls -la /dev/kvm

# Charger le module (si disponible)
modprobe kvm
modprobe kvm_intel  # ou kvm_amd
```

### Port déjà utilisé

```bash
# Trouver le processus
lsof -i :8080

# Tuer le processus
kill -9 <PID>
```

### Firecracker ne démarre pas

```bash
# Vérifier les permissions
chmod 644 /var/lib/firecracker/vmlinux
chmod 644 /var/lib/firecracker/rootfs.ext4

# Vérifier l'espace disque
df -h
```

---

## Résumé des fichiers

| Fichier | Chemin | Description |
|---------|--------|-------------|
| Firecracker | `/usr/local/bin/firecracker` | Binaire Firecracker |
| Kernel | `/var/lib/firecracker/vmlinux` | Kernel Linux 5.10 |
| Rootfs | `/var/lib/firecracker/rootfs.ext4` | Ubuntu 22.04 rootfs |
| Server | `/usr/local/bin/firecracker-server` | API serveur |
| Service | `/etc/systemd/system/firecracker-server.service` | Service systemd |
| Logs | `/var/log/firecracker-server.log` | Logs du serveur |

---

## Résumé des ports

| Port | Service | Accès |
|------|---------|-------|
| 22 | SSH | Votre IP uniquement |
| 8080 | firecracker-server | Votre IP uniquement |

---

## Commandes utiles

```bash
# Status du serveur
systemctl status firecracker-server

# Voir les logs en temps réel
journalctl -u firecracker-server -f

# Redémarrer le serveur
systemctl restart firecracker-server

# Arrêter le serveur
systemctl stop firecracker-server

# Test rapide depuis le serveur
curl localhost:8080/health
```
