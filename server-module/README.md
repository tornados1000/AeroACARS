# server-module/

phpVMS 7 module **CloudeAcars** — drop-in into a phpVMS install's `modules/` folder.

**Status:** Phase 0 — placeholder. Module scaffold will be created in Phase 4.

## Planned layout

```
server-module/
└── CloudeAcars/
    ├── module.json
    ├── composer.json
    ├── Config/
    │   └── config.php
    ├── Database/
    │   ├── migrations/
    │   │   ├── *_create_cloudeacars_config.php
    │   │   ├── *_create_cloudeacars_pirep_extra.php
    │   │   └── *_create_cloudeacars_client_versions.php
    │   └── seeds/
    ├── Http/
    │   ├── Controllers/
    │   │   ├── Admin/AdminController.php
    │   │   └── Api/ApiController.php
    │   └── Resources/
    ├── Models/
    │   ├── Config.php
    │   ├── PirepExtra.php
    │   └── ClientVersion.php
    ├── Providers/
    │   ├── CloudeAcarsServiceProvider.php
    │   └── EventServiceProvider.php
    ├── Resources/
    │   ├── views/
    │   └── lang/{de,en}/
    └── Services/
        └── CloudeAcarsService.php
```

## API surface (Phase 4)

| Method | Path | Auth |
|---|---|---|
| `GET` | `/api/cloudeacars/config` | `api.auth` |
| `GET` | `/api/cloudeacars/version` | public |
| `POST` | `/api/cloudeacars/heartbeat` | `api.auth` |
| `POST` | `/api/cloudeacars/pirep/{id}/landing` | `api.auth` |
| `POST` | `/api/cloudeacars/runway-data/missing` | `api.auth` |

For everything else (users, bids, flights, fleet, PIREP file, ACARS positions) the client uses **phpVMS Core API** directly.
