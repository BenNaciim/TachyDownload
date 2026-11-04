# Réécriture Rust + Tauri : plan d'exécution (document vivant)

Conception : [`../specs/2026-11-04-rust-tauri-rewrite-design.md`](../specs/2026-11-04-rust-tauri-rewrite-design.md) (validée). La conception dit *quoi* ; ce fichier dit *dans quel ordre* et *où on en est*. Il est mis à jour à chaque tâche terminée, pour que le travail survive à une coupure de session.

## Règles de travail

- Branche `rust-tauri`. `main` reste l'Electron 1.x tant que la 2.0.0 n'est pas à parité. Rien n'est poussé sur `main`, aucune étiquette ni release avant M7.
- TDD pour tout code produit : test rouge (vu échouer pour la bonne raison), code minimal, test vert, suite complète du périmètre verte. Les sondes (S1 à S5) sont jetables et minimales.
- Un commit local par tâche verte, messages en anglais comme le dépôt, fin de message : `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Branche poussée en fin de jalon.
- Tant que le JS existe : `npm test` (152 tests) et `npm run lint` doivent rester verts. Les tests JS ne sont supprimés qu'une fois leur équivalent Rust vert (§7.2 de la conception).
- Le comportement de référence est le code JS de `src/` et ses tests. En cas de doute, le JS gagne (parité, pas d'amélioration).
- Les textes français sont repris mot pour mot (conception §5.5).
- Tout résultat chiffré ou toute surprise va dans la conception §10.4 (preuves) ou dans le journal ci-dessous le jour même.

## Commandes

```bash
cargo build -p tachy-download            # application (fenêtre Tauri)
cargo test --workspace                   # tests Rust
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
npm test                                 # tests JS (Electron actuel, extension, pont)
npm run lint
npm run dist                             # installeur Electron 1.x (référence S0)
```

## Jalons

### M0 Bases et preuves

- [x] Branche `rust-tauri`, interface déplacée dans `ui/`, Electron repointé, 152 tests verts
- [x] Pont `ui/renderer/dm-bridge.js` + `test/bridge.test.js` (9 tests)
- [x] Workspace Cargo (CRT statique, outils épinglés) qui compile
- [x] Fenêtre Tauri qui affiche l'interface avec un faux noyau (capture CDP : liste, graphique, événements `dm:update`)
- [x] S0 référence 1.2.0 : installeur 107,7 Mio, RAM au repos 205,4 Mio, démarrage 623 ms, banc Node (conception §10.4)
- [x] S2 HTTP/1.1 Range : GO (8 connexions : 1 979 Mo/s contre 306 pour Node, 2,6× moins de CPU) · S3 `safeStorage` `v10` : GO · S5 icônes : GO
- [x] S1 verre : GO avec `--disable-gpu-compositing` (défaut WebView2 156 sinon) · S4 installeur : GO (1,83 Mio ; installation, mise à jour `/UPDATE /R` avec relance, désinstallation) · S5 notification : demi-GO (API `on_activated` présente, clic réel à vérifier à la main en M5, repli sans clic)
- [x] Mesure Tauri release (faux noyau, au repos) : 122 Mio, 516 ms, 5,3 % CPU contre Electron 196 Mio, 607 ms, 2,2 % (conception §10.4)
- [x] Document de parité créé (`../specs/2026-11-04-rust-tauri-rewrite-parity.md`) ; résultats consignés en §10.4 pour S0 à S5
- [ ] Pousser la branche (fin de jalon) puis rapport M0

### M1 Noyau (`crates/tachy-core`)

Ordre (chaque module : tests JS portés d'abord, puis code) :

- [ ] `util` (noms de fichiers, types, catégories) · `errors` · `fsutil`
- [ ] `settings` (défauts, normalisation, fenêtres horaires) · `rules`
- [ ] `limiter` (jetons) · `store` (`downloads.json`, écriture atomique, copie `.illisible`)
- [ ] `net` (proxy, client HTTP/1.1, en-têtes, cookies, redirections) · `probe`
- [ ] `task/http` (segments, vol, reprise, tentatives, connexions adaptatives, liens expirés)
- [ ] `manager` (acteur : file, planification, nouvelles tentatives, `redownload`, `refresh`)
- [ ] `api` (routes de l'extension, jeton, CORS) · instantané `list()`
- [ ] Contrats : `tools/make-fixtures.js`, test Rust des fixtures ; banc `crates/tachy-core/examples/bench.rs` ≥ référence

### M2 HLS · M3 Vidéos · M4 Coque · M5 Données et mise à jour · M6 Installeur · M7 Livraison

Détail par jalon dans la conception §8. Les cases seront dépliées ici au début de chaque jalon.

## Journal

- 2026-11-04 : conception validée ; autorisation donnée de tout faire sans s'arrêter (installeur d'essai compris). M0 étapes 1 et 2 faites (`be9ffc1`).
- 2026-11-04 : S1 : le compositeur GPU de WebView2 156 (portable à deux GPU) laisse des couches `backdrop-filter` à moitié peintes après un changement de style en direct ; contournements côté interface sans effet, `--disable-gpu-compositing` règle tout (RAM au repos 122 Mio au lieu de 223, CPU au repos 5,3 % au lieu de 0,8 %). À refaire avec le vrai noyau en M7.
- 2026-11-04 : un `cargo build --release` est resté bloqué 47 min (rustc quasi inactif) : tuer l'arbre de processus et relancer suffit (2 min).
- 2026-11-04 : `rust-toolchain.toml` épingle `1.99.1` (rustup télécharge la chaîne au premier appel). `.cargo/config.toml` : `+crt-static`. Icônes lues depuis `build/` pour l'instant (déplacées en M7).
