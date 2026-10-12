# WebSDR

Browserbasierter Empfänger für RTL-SDR-Sticks (getestet für den **RTL-SDR Blog V4**).
Die Bedienung ist an SDR# angelehnt: Mittenfrequenz und Empfangsfrequenz lassen sich frei
einstellen, es gibt keine festen Profile wie bei OpenWebRX.

- Backend: Rust (axum, rustfft), Treiber: librtlsdr aus dem rtl-sdr-blog-Fork (V4-Unterstützung)
- Frontend: Svelte + TypeScript, Wasserfall in WebGL2, Audio über AudioWorklet
- Läuft als Docker-Container auf Ubuntu und unter Windows (WSL2)

## Schnellstart

```bash
# im Projektordner (unter Windows: im WSL-Ubuntu-Terminal)
docker compose up -d --build
```

Danach im Browser **http://localhost:8073** öffnen und oben links auf **▶ Start** klicken
(Browser spielen Audio erst nach einem Klick ab).

Ohne Stick zum Ausprobieren mit eingebautem Simulator:

```bash
SDR_SOURCE=sim docker compose up -d --build
```

Der Simulator erzeugt Sender um 100 MHz: WFM 100,0 MHz, NFM 100,3 MHz (getastet),
AM 99,5 MHz, USB 99,3 MHz und CW 99,2 MHz.

## Bedienung

| Aktion | Wirkung |
|---|---|
| Klick in Spektrum/Wasserfall | VFO auf diese Frequenz (gerastet auf die Schrittweite, abschaltbar) |
| Mausrad im Spektrum | VFO ± Schrittweite |
| Strg + Mausrad | Zoom um die Mausposition |
| Umschalt + Mausrad | Mittenfrequenz verschieben |
| Ziehen mit der Maus | Mittenfrequenz verschieben (Stick wird umgestimmt) |
| Filterkante ziehen | Bandbreite ändern |
| Mausrad über einer Ziffer der Frequenzanzeige | genau diese Stelle ändern |
| Klick obere/untere Hälfte einer Ziffer | Stelle +1/−1 |
| ✎ oder Doppelklick auf die Frequenz | Eingabe, z. B. `145.5M`, `7100k`, `433.92` |
| ← / → | VFO ± Schritt (mit Umschalt ×10) |
| ↑ / ↓ | VFO ± 10 Schritte |
| Leertaste | Audio an/aus |
| M | Stumm |

Liegt der VFO außerhalb des empfangenen Bandes, stimmt der Server die Mitte automatisch
nach (wie SDR#). Die Einstellungen werden in `./data/settings.json` gespeichert,
Lesezeichen in `./data/bookmarks.json`.

Mehrere Browser können gleichzeitig zuhören, jeder mit eigenem VFO innerhalb des Bandes.
Mitte, Abtastrate und Verstärkung ändert nur der Browser mit **Steuerrecht** (der erste,
der sich verbindet; übernehmen per Knopf in der Seitenleiste, optional mit
`CONTROL_PASSWORD`).

## Konfiguration

Umgebungsvariablen (in `docker-compose.yml` oder per `.env`-Datei):

| Variable | Standard | Bedeutung |
|---|---|---|
| `SDR_SOURCE` | `usb` | `usb`, `usb:1` (zweiter Stick), `rtl_tcp://host:port`, `sim` |
| `WEBSDR_PORT` | `8073` | Port auf dem Host |
| `CONTROL_PASSWORD` | leer | Passwort für das Steuerrecht (leer = erster Browser steuert) |

## Ubuntu

Der DVB-T-Treiber des Kernels belegt den Stick sonst. Einmalig:

```bash
echo 'blacklist dvb_usb_rtl28xxu' | sudo tee /etc/modprobe.d/blacklist-rtl.conf
sudo modprobe -r dvb_usb_rtl28xxu
docker compose up -d --build
```

## Windows (WSL2 + usbipd)

Docker läuft in WSL-Ubuntu. Der Stick wird mit [usbipd-win](https://github.com/dorssel/usbipd-win)
an WSL weitergereicht:

```powershell
# PowerShell als Administrator, einmalig:
usbipd list                        # BUSID des Sticks suchen (0bda:2838)
usbipd bind --busid <BUSID>

# nach jedem Einstecken oder Neustart (ohne Admin):
usbipd attach --wsl --busid <BUSID>
# oder automatisch wieder anhängen, solange das Fenster offen ist:
usbipd attach --wsl --auto-attach --busid <BUSID>
```

Dann im WSL-Ubuntu-Terminal im Projektordner `docker compose up -d --build` und unter
Windows im Browser http://localhost:8073 öffnen.

**Ausweichweg ohne usbipd:** `rtl_tcp.exe` aus dem
[rtl-sdr-blog-Release](https://github.com/rtlsdrblog/rtl-sdr-blog/releases) unter Windows
starten (`rtl_tcp -a 0.0.0.0`) und den Container mit
`SDR_SOURCE=rtl_tcp://host.docker.internal:1234` starten.

## Fehlersuche

- Oben rechts steht **Kein Empfänger** → Seitenleiste „Quelle“ zeigt die Ursache.
  `http://localhost:8073/health` liefert dieselbe Information als JSON.
- `docker compose logs -f` zeigt das Server-Log.
- „kein RTL-SDR gefunden“: Stick eingesteckt? Unter Windows per `usbipd attach --wsl` weitergereicht?
  In WSL prüfen mit `lsusb` (muss `0bda:2838` zeigen).
- „Verworfene Blöcke“ in der Seitenleiste: Die CPU kommt nicht hinterher, Abtastrate senken.

## Entwicklung

```bash
# Backend (Simulator, ohne Docker)
cd backend && SDR_SOURCE=sim WEBSDR_STATIC=../frontend/dist cargo run --release
cargo test --release                      # DSP-Tests
cargo test --release bench -- --ignored --nocapture   # Rechenzeit je Modus

# Frontend mit Hot-Reload (leitet /ws an das Backend auf :8073 weiter)
cd frontend && npm install && npm run dev
```
