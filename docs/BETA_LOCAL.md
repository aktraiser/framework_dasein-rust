# Lancer le laboratoire local

Aucun serveur SSH ni compte LLM n'est nécessaire pour cette démonstration. Docker Desktop doit fonctionner. Depuis la racine du dépôt :

```bash
docker pull python:3.12-alpine
docker run -d --name dasein-lab-nats -p 127.0.0.1:4222:4222 -v dasein-lab-nats:/data nats:2.10.26-alpine -js -sd /data
cargo run --locked --example durable_lab --features docker -- publish lab premiere-tache
cargo run --locked --example durable_lab --features docker -- work lab premiere-tache
cargo run --locked --example durable_lab --features docker -- status lab premiere-tache
```

Résultat attendu : `Completed: 42`, puis un résultat persistant avec `stdout: "42\n"`. On peut lancer plusieurs processus `work`; un seul réclame chaque livraison. Une fois terminée, une tâche conserve son identifiant : utiliser un nouvel identifiant pour une nouvelle exécution. L'agent garde sa mémoire via les stockages du SDK ; la queue transporte le travail et conserve son résultat. Redis est optionnel pour cette démonstration.

Arrêt sans effacer le stockage : `docker stop dasein-lab-nats`. Reprise : `docker start dasein-lab-nats`. Ne pas recréer un conteneur avec le même nom. Les tests automatisés nécessitent des services **distincts** de ce laboratoire : ils suppriment leurs streams.

## Gateway Python

Dans un terminal à la racine du dépôt :

```bash
export GATEWAY_API_KEY="$(openssl rand -hex 32)"
export GATEWAY_CONFIG="$PWD/gateway/beta-local.toml"
export DATABASE_URL="sqlite:$PWD/gateway/gateway.db?mode=rwc"
cargo run --locked --manifest-path gateway/Cargo.toml -p gateway-server
```

Dans un autre terminal, utiliser le même token, sans le publier ni le mettre dans Git. Les appels suivants montrent le contrat ; remplacer `BOX_ID` par l'identifiant retourné :

```bash
curl -sS http://127.0.0.1:8080/v1/sandboxes \
  -H "Authorization: Bearer $GATEWAY_API_KEY" -H 'Content-Type: application/json' \
  -d '{"runtime":"python","timeout_seconds":30,"memory_mb":256}'
curl -sS http://127.0.0.1:8080/v1/sandboxes/BOX_ID/execute \
  -H "Authorization: Bearer $GATEWAY_API_KEY" -H 'Content-Type: application/json' \
  -d '{"code":"print(6*7)","runtime":"python"}'
curl -sS -X DELETE http://127.0.0.1:8080/v1/sandboxes/BOX_ID \
  -H "Authorization: Bearer $GATEWAY_API_KEY"
```

La propriété et les limites de la Box survivent au redémarrage de la gateway. Les fichiers créés dans une exécution sont supprimés à sa fin. Le profil actuel exécute Python dans Docker. Le [profil Firecracker séparé](../server/firecracker-guest/README.md) est désormais testé avec KVM dans une VM Linux locale sur Mac M4 ; cette gateway conserve Docker. Les autres langages restent à qualifier.

Pour activer un fournisseur LLM, configurer le tableau `llm.providers` et les routes `llm.models`, ou utiliser le dashboard avec `GATEWAY_ADMIN_PASSWORD` et `GATEWAY_MASTER_KEY`. Les appels réels seront facturés par le fournisseur ; la démonstration ci-dessus n'en effectue aucun. Ne pas publier cette gateway sur Internet avant les validations supplémentaires du document de statut.

## L'accès SSH, plus tard

SSH est l'accès à un ordinateur Linux distant. Si aucun serveur n'a été créé, il n'y a pas encore d'accès à fournir. Si un serveur existe déjà, son espace d'administration fournit son adresse IP, et l'image Linux choisie détermine le compte de connexion. L'hébergeur indique généralement la commande `ssh utilisateur@adresse-IP`. Fournir uniquement cette commande ou un alias existant, jamais le mot de passe ou la clé privée.

Avant de louer quoi que ce soit pour Firecracker, il faut vérifier que la machine permet réellement l'accès à `/dev/kvm` (un VPS quelconque ne le garantit pas). Le laboratoire Docker local permet de continuer pendant cette étape. Aucun serveur payant n'a été créé par cette intervention.
