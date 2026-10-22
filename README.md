# Edge Shell

Nakładka na Windows, która zostawia środek ekranu pracy, a wszystko inne trzyma na krawędziach. Plan dnia bierze z [Horologa](https://github.com/lukmack7/horolog).

- **Górny pasek.** Kolorowe klocki dnia z Horologa z kreską „teraz”, timer fokusu, tryby Praca / Nauka / Wieczór, radio, muzyka, bateria i zegar. Pasek rezerwuje miejsce na ekranie, więc zmaksymalizowane okna go nie zasłaniają.
- **Dock.** Pod każdą ikoną jest pasek koloru kategorii, a kropka pokazuje uruchomione aplikacje. W bieżącym trybie część aplikacji jest wyciszona; kliknięcie takiej otwiera **żółtą flagę**: „Wracam do pracy”, „Do bufora” albo „Otwórz mimo to” po 5 sekundach.
- **Szuflada (lewa krawędź).**
  - Wyszukiwarka z kalkulatorem.
  - **Półka**: przeciągnij plik na szufladę, a poczeka tam, aż znajdziesz mu miejsce.
  - Przypięte foldery, ostatnie pliki i historia schowka.
  - **Garaż**: odkładasz wszystkie otwarte okna jako jeden kontekst i przywracasz je jednym kliknięciem.
- **Teraz (prawa krawędź).**
  - Bieżący blok z Horologa z pierścieniem czasu oraz przyciski Pauza / Skończone / Przeplanuj.
  - „Wrzuć myśl”, czyli zapis językiem naturalnym prosto do Horologa.
  - Plan dnia, tygodniowy cel fokusu i Spotify.
- **Radio.** Wszystkie powiadomienia Windows i wiadomości z lokalnego endpointu wyglądają jak rozmowy z pit wall: numer startowy, nazwa, cytat i gotowa odpowiedź na jeden klik.
  - W trybie cichym zwykłe wiadomości czekają na paczkę o ustalonej godzinie.
  - Od razu przechodzą tylko pilne wiadomości i kontakty z numerem 1–9.
- **Pit wall.** Komunikaty o starcie bloków, przypomnienie o spotkaniu 10 minut wcześniej i „Box, box” przed buforem. Krawędź ekranu świeci na bursztynowo przed spotkaniem.
- **Sektory.** Klik „Skończone” koloruje blok: fioletowy, jeśli skończyłeś przed planem, zielony w czasie, żółty po czasie.
- **Dymki.** Gdy panel Teraz jest schowany, przy prawej krawędzi zostają timer i muzyka.

## Instalacja

1. Pobierz instalator z zakładki **Actions** → ostatni udany build → artefakt `edge-shell-installer`. Docelowo będzie też w **Releases**.
2. Uruchom `Edge Shell_x.y.z_x64-setup.exe`. Instaluje się dla bieżącego użytkownika, bez uprawnień administratora.
3. Przy pierwszym starcie powstaje `%APPDATA%\EdgeShell\config.json`. Wpisz tam adres swojego Horologa i kliknij **Wczytaj ponownie** na dole panelu Teraz.

Jeśli używasz MyDockFinder, wyłącz jego dock i górny pasek, żeby nie dublowały się z Edge Shell. Możesz też zostawić MyDockFinder i w Edge Shell ustawić `shell.showDock: false`.

## Skróty

Windows zajmuje Win+strzałki, Win+K i Win+Space, dlatego domyślne skróty są na Ctrl+Alt. Zmienisz je w `hotkeys` w konfiguracji.

| Skrót | Akcja |
| --- | --- |
| Ctrl+Alt+← | Szuflada |
| Ctrl+Alt+→ | Teraz |
| Ctrl+Alt+K | Wrzuć myśl |
| Ctrl+Alt+F | Start / pauza fokusu |
| Ctrl+Alt+Space | Szukaj w szufladzie |
| Ctrl+Alt+R | Pokaż radio z paczki |
| Esc | Schowaj okno |

Panele wysuwają się też same, gdy kursor dotknie lewej albo prawej krawędzi ekranu na `shell.edgeDelayMs` milisekund (domyślnie 150).

## Konfiguracja (`%APPDATA%\EdgeShell\config.json`)

```jsonc
{
  "horolog": {
    "url": "http://homeserver.local:8000",      // API Horologa
    "webUrl": "http://homeserver.local:3000"    // interfejs WWW, otwierany z docka
  },
  "startMode": "praca",
  "modes": [
    {
      "id": "praca", "label": "Praca", "accent": "#D42E25",
      "dim": ["discord", "youtube", "steam"],  // id aplikacji z docka, które gasną
      "quiet": true,                           // zwykłe radio czeka na paczkę
      "playlist": "spotify:playlist:..."       // co grać na start fokusu
    }
  ],
  "dock": [
    {
      "id": "excel", "name": "Excel", "path": "excel.exe",
      "category": "projekty",   // praca | projekty | komunikacja | tworcze | system
      "icon": "sheet",          // albo 1–2 litery
      "process": "EXCEL.EXE",
      "separatorAfter": false
    }
  ],
  "pinned": [ { "name": "Pobrane", "path": "%USERPROFILE%\\Downloads", "category": "praca" } ],
  "contacts": [ { "name": "KASIA", "number": "7", "color": "#fbbf24", "matches": ["Kasia", "Katarzyna"] } ],
  "radio": { "port": 7311, "token": "", "batchTimes": ["10:00", "12:00", "15:00", "17:00"] },
  "spotify": { "clientId": "", "redirectPort": 7312, "focusVolume": 30 },
  "shell": { "reserveTopBar": true, "hideTaskbar": false, "showDock": true, "showBubbles": true, "edgeDelayMs": 150, "edgeGlowMinutes": 15 },
  "focus": { "adhocMinutes": 25, "weeklyGoalHours": 15 }
}
```

- **`dock[].path`** przyjmuje kilka rodzajów wpisów:
  - pełną ścieżkę do `.exe`,
  - nazwę z App Paths (`excel.exe`, `chrome.exe`),
  - URI (`spotify:`, `steam://open/main`, `ms-settings:`),
  - adres WWW.
- **Ikony** do wyboru: `folder`, `globe`, `sheet`, `terminal`, `mail`, `clock`, `chat`, `note`, `music`, `video`, `game`, `code`, `image`, `calc`, `settings`.
- **`shell.hideTaskbar`** chowa pasek zadań Windows. To funkcja eksperymentalna: Windows 11 potrafi go przywrócić. Edge Shell sam pokazuje go z powrotem przy wyjściu.

## Horolog

Edge Shell korzysta z tych tras API Horologa (bez logowania, więc trzymaj Horologa w sieci domowej albo za VPN):

| Trasa | Do czego |
| --- | --- |
| `GET /api/plan` | Bloki i zajęte terminy na dziś |
| `GET /api/stream` | Sygnał o zmianie planu (SSE), po którym plan jest pobierany od nowa |
| `GET /api/analytics` | Minuty fokusu do paska celu |
| `POST /api/capture` | „Wrzuć myśl” |
| `POST /api/intents/{id}/complete` | „Skończone” dla jednorazowych zadań |
| `POST /api/plan/solve` | „Przeplanuj” |

Ostatni plan jest zapisywany lokalnie, więc panel działa też bez sieci. Myśli wrzucone offline czekają w kolejce i wysyłają się po powrocie połączenia.

## Radio: wysyłanie wiadomości

Lokalny endpoint nasłuchuje tylko na `127.0.0.1`. Możesz go zasilać z n8n, Home Assistanta, skryptu albo telefonu przez przekaźnik.

```powershell
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:7311/radio -ContentType 'application/json' -Body (@{
  from     = 'Kasia'
  source   = 'komunikator'
  message  = 'Masz chwilę po 15? Mam pytanie do raportu'
  reply    = 'Tak, po spotkaniu. Wpadnę o 15:30'
  replyUrl = 'http://n8n.local/webhook/odpowiedz'   # opcjonalnie
  urgent   = $false
} | ConvertTo-Json)
```

- Bez `replyUrl` przycisk „Wyślij odpowiedź” kopiuje odpowiedź do schowka.
- Z `replyUrl` odpowiedź idzie jako `POST {"reply": "...", "id": "...", "to": "..."}`.
- Jeśli ustawisz `radio.token`, dodawaj nagłówek `Authorization: Bearer <token>`.

## Radio: wszystkie powiadomienia Windows

Edge Shell przechwytuje powiadomienia **wszystkich aplikacji**: Teams, Outlook, WhatsApp, Discord, Telefon, a także powiadomienia stron z przeglądarki.

- Windows zapisuje każde powiadomienie w bazie `%LOCALAPPDATA%\Microsoft\Windows\Notifications\wpndatabase.db`. Edge Shell czyta ją wyłącznie do odczytu, co sekundę.
- Pierwsza linia powiadomienia staje się nadawcą (dopasowywanym do `contacts`), reszta treścią.
- Przycisk **Otwórz** uruchamia aplikację, z której przyszło powiadomienie.

**Ważne: włącz w Windows „Nie przeszkadzać”** (Ustawienia → System → Powiadomienia). Systemowe dymki znikną, ale powiadomienia dalej trafiają do bazy, więc zobaczysz je tylko jako radio. Bez tego każde powiadomienie pojawi się dwa razy.

Reguły dla aplikacji ustawiasz w `notifications.apps`:

```jsonc
"notifications": {
  "enabled": true,
  "apps": [
    { "matches": "Spotify", "ignore": true },                      // pomijaj
    { "matches": "Teams", "color": "#a5b4fc" },                    // własny kolor
    { "matches": "Outlook", "urgent": true, "name": "Poczta" }     // zawsze od razu
  ]
}
```

Pole `matches` to fragment identyfikatora aplikacji albo jej nazwa.

## Spotify

1. Na [developer.spotify.com](https://developer.spotify.com/dashboard) utwórz aplikację. Jako Redirect URI dodaj `http://127.0.0.1:7312/callback`.
2. Wpisz jej Client ID w `spotify.clientId`, wczytaj konfigurację ponownie i kliknij **Połącz** w panelu Teraz.
3. Reguły ustawiasz w konfiguracji:
   - start fokusu włącza playlistę trybu ściszoną do `focusVolume`,
   - bufor pauzuje muzykę,
   - 2 minuty przed spotkaniem muzyka się wycisza,
   - zmiana trybu przełącza playlistę.

Sterowanie odtwarzaniem przez Web API wymaga konta Premium. Bez niego widget tylko pokazuje, co gra.

## Budowanie ze źródeł

Potrzebujesz Windows 10/11, [Rust](https://rustup.rs), Node 20+ i WebView2 (jest w Windows 11).

```powershell
npm install
npm run dev     # uruchomienie w trybie deweloperskim
npm run build   # instalator w src-tauri\target\release\bundle\nsis
```

Frontend to zwykłe pliki HTML i JS w `ui/`, bez kroku budowania. Backend w Rust (`src-tauri/src`) składa się z modułów:

| Moduł | Zakres |
| --- | --- |
| `horolog` | Most do Horologa |
| `layout` | Okna, AppBar, krawędzie |
| `platform` | Wywołania Win32 |
| `radio` | Radio |
| `spotify` | Spotify |
| `engine` | Reguły czasu, schowek, aktywne okno |
| `commands` | Komendy wywoływane z okien |
