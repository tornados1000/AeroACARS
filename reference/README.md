# Reference Assets / Referenz-Material

**EN:** Read-only reference material from the existing closed-source vmsACARS product. Used to ensure API compatibility and as a baseline reference for our own implementation. **Do not modify.** Do not import any code from the binary installer — only the publicly distributed phpVMS server module is permitted as a structural reference for our new `CloudeAcars` server module.

**DE:** Read-only-Referenzmaterial aus dem bestehenden Closed-Source-Produkt vmsACARS. Wird zur Sicherstellung der API-Kompatibilität und als Strukturreferenz für unsere eigene Implementierung verwendet. **Nicht modifizieren.** Aus dem Windows-Installer-Binary darf kein Code übernommen werden — nur das öffentlich verteilte phpVMS-Server-Modul ist als strukturelle Referenz für unser neues `CloudeAcars`-Modul zulässig.

## Contents / Inhalt

| Path | Description |
|---|---|
| `vmsacars-web-stable.zip` | Original distribution archive of the vmsACARS phpVMS server module |
| `vmsacars-web-extracted/VMSAcars/` | Extracted phpVMS module — public, used as API/structural reference |
| `vmsacars-win-x64-stable-Setup.exe` | Closed-source Windows installer of the existing client. Binary only. **No source available, no decompilation, no reuse.** |

## Why kept here / Warum hier abgelegt

- **API compatibility:** Our new client should be able to talk to phpVMS sites that have the original `VMSAcars` module installed (transitional period).
- **Field naming:** Helps us match phpVMS PIREP custom-field naming conventions where it makes sense, so VAs can switch back and forth.
- **Database migrations:** Shows which tables already exist if a site upgrades from `VMSAcars` to `CloudeAcars`.
