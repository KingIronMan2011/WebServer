# KingIronMan Linux package repository

This image serves a persistent Dokploy volume mounted at `/repo`.

## Storage layout

```text
/repo
├── debian/              # Debian and Ubuntu APT repository trees
├── rpm/fedora/          # Fedora RPM trees, including repodata/
├── arch/x86_64/         # pacman packages and *.db.tar.zst
├── keys/                # public signing keys only
└── .repository-initialized
```

The image copies its initial APT repository to the volume only when
`.repository-initialized` is absent. Every later deploy serves the existing
volume, so package artefacts and metadata persist.

## Dokploy

Attach the named Docker volume `kingironman-package-repository` at `/repo`.
Keep the service at one replica because metadata generators write shared indexes.

## Publishing contract

A publisher must upload native, signed packages and update the matching index
atomically:

- Debian/Ubuntu: publish under `debian/` using `reprepro` or equivalent.
- Fedora: publish under `rpm/fedora/<release>/<arch>/`, then run
  `createrepo_c --update`.
- Arch: publish under `arch/<arch>/`, then run `repo-add --include-sigs`.

Do not put private signing keys in this repository volume or the serving image.
