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

## Bezpiecznik, dock i kopia ustawień

- **Bezpiecznik** — razem z Edge Shell startuje mały proces strażnika (ten sam plik `.exe` z `--watchdog`). Jeśli Edge Shell padnie albo zawiesi się na ~15 s, strażnik przywraca pasek zadań Windows i każe programom ponownie zgłosić ikony zasobnika. Po odwieszeniu Edge Shell wraca do swojego układu.
- **Autoukrywanie docka** — dock wysuwa się od razu po dotknięciu dolnej krawędzi pod nim i chowa pół sekundy po odjechaniu kursora (nie chowa się, gdy są otwarte podglądy okien albo stos).
- **Pełny ekran** — gra, film albo prezentacja na pełnym ekranie chowa pasek, lewy pasek, dock i dymki na tym monitorze; krawędzie nie wysuwają wtedy paneli.
- **Plakietki** — powiadomienie z aplikacji, która ma ikonę w docku, dokłada licznik na ikonie. Przejście do aplikacji zdejmuje plakietkę.
- **Stosy** — folder dodany do docka (*Dodaj aplikację → Folder jako stos…*) po kliknięciu pokazuje siatkę najnowszych plików. Shift+klik otwiera folder w Eksploratorze.
- **Kopia ustawień** — każdy zapis w ustawieniach tworzy kopię w `%APPDATA%\EdgeShell\backups` (ostatnie 20) i — jeśli wskazano folder, np. udział na home serwerze — plik `EdgeShell-<komputer>.json` w tym folderze. W *Ustawienia → Ogólne* jest też eksport, import i przywracanie.

## Paleta poleceń

`Ctrl+Alt+Space` albo lupa na lewym pasku. Jedno pole na wszystko:

- **szybkie klawisze** — `fk 45` fokus na 45 min, `fb` fokus na bieżący blok, `fp` pauza, `tp` / `tn` / `tr` tryby, `m treść` myśl do Horologa, `og` odłóż okna, `sp` / `sn` Spotify, `ts` / `tx` Tailscale exit node, `= 12*3` kalkulator, `pl` przeplanuj, `rap` raport, `ust` ustawienia (klawisze zmienisz w *Ustawienia → Skróty*),
- **wyszukiwanie** — otwarte okna (Enter przełącza zamiast otwierać drugi raz), aplikacje z docka i menu Start, pliki z Pulpitu, Dokumentów, Pobranych i przypiętych folderów, bloki z Horologa,
- **Tab** — więcej akcji dla wyniku (otwórz folder, kopiuj ścieżkę, na półkę, zamknij okno, oznacz jako zrobione…),
- gdy nic nie pasuje, ostatni wynik wrzuca wpisany tekst do Horologa.

## Automatyzacje

*Ustawienia → Automatyzacje*: reguły „Gdy → to”. Warunki: czas (dni tygodnia), monitor (po nazwie z EDID albo dowolny dodatkowy), sieć Wi‑Fi, nowy plik w folderze (wzorzec, np. `*.pdf`), blok z Horologa (spotkanie, fokus…, X min wcześniej), aplikacja / pełny ekran, bateria, bezczynność. Akcje wykonują się po kolei: tryb, Spotify, cisza radia, karta radia, półka, odłożenie / zamknięcie okien aplikacji, uruchomienie, jasność, głośność, fokus, schowanie docka i pasków, dowolne polecenie `cmd`. Opcje: *najpierw zapytaj* (karta radia z „Tak / Nie”), *nie częściej niż*, *gdy warunek minie, przywróć*. Dziennik pokazuje, co się wykonało. Przykładowe reguły są dodane, ale wyłączone.

## Odtwarzacz na wierzchu

Pływająca karta Spotify z okładką; tło, tytuł, pasek postępu i przycisk play biorą kolory z okładki. Wyciągasz ją klikiem w tytuł w górnym pasku (klik w korektor = play/pauza), przytrzymaniem dymka muzyki, `sp` w palecie albo `Ctrl+Alt+M`. Przeciągasz za okładkę (przy krawędzi przyciąga się do rogu), podwójny klik zwija kartę do paska, kółko myszy zmienia głośność, 📌 odpina z wierzchu. Bez kursora przygasa, w grach się chowa, na początku fokusu sam zwija się do paska. Pozycja i rozmiar są zapamiętywane.

## Pasek i lewy pasek — karty

- **CPU / RAM** → co najbardziej obciąża (procesy pogrupowane po programie) z przyciskiem *Zakończ* (dwa kliknięcia; procesów systemowych nie da się zakończyć).
- **Wi‑Fi** → lista sieci; zapisane łączą się od razu, nowe pytają o hasło.
- **Bluetooth** → sparowane urządzenia z poziomem baterii (gdy Windows go zna).
- **Dźwięk** → głośność, wybór wyjścia (głośniki / słuchawki) i głośność każdej aplikacji osobno. Środkowy klik wycisza, kółko zmienia głośność.
- **Zasilanie** (na dole lewego paska) → zablokuj, uśpij, hibernuj, uruchom ponownie, wyłącz, wyloguj. Restart / wyłączenie / wylogowanie wymagają drugiego kliknięcia.

## Dock — więcej

Kółko myszy nad ikoną przełącza okna tej aplikacji. Ikony przeciągasz bezpośrednio na docku (przeniesiona ikona przejmuje sekcję sąsiada). Plik upuszczony z Eksploratora na ikonę otwiera się w tej aplikacji (na stosie — kopiuje się do folderu); upuszczony na pusty fragment docka zostaje przypięty.

## Fokus i ludzie

- **Jedna rzecz** (`Ctrl+Alt+J`, `jr` w palecie): wszystko poza aktywnym oknem przygasa, dock się chowa, pasek pokazuje tylko zadanie.
- **Rozproszenia**: kilka minut w YouTube / Discordzie w trakcie fokusu → karta „Wracamy do …?” (*Wracam* przełącza do okna pracy). Bez blokowania.
- **Start bloku z pierwszym krokiem**: karta „Zielone światło” pokazuje 2‑minutowy pierwszy krok (ustawiasz go w panelu Teraz), z przyciskami *Jadę* / *Za 5 min*.
- **Zamknięcie dnia** (pon–pt o ustawionej porze albo `kd`): co zrobione, niedokończone jednym kliknięciem na jutro, jedno zdanie o dniu.
- **Prawdziwe czasy**: gdy nawyk trzy razy z rzędu trwa wyraźnie dłużej/krócej niż w planie, karta proponuje poprawkę czasu w Horologu.
- **Praca razem**: serwer obecności w `server/presence` (Python bez zależności, `docker compose up -d` na home serwerze). Osoby z tym samym adresem i tokenem widzą się w górnym pasku (kto w fokusie, do kiedy).

## Kalendarz pod zegarem

Dzień jest osią czasu — blok przeciągasz na inną godzinę albo na dzień w kalendarzu. Edge Shell ustawia w Horologu preferowaną godzinę i dzień, a Horolog dobiera najbliższe wolne miejsce.

## Pliki, schowek, zrzuty

- **Podgląd**: `Shift+Spacja` na pliku w palecie, `Spacja` w stosie — obrazy, tekst, a PDF / Office / wideo jako miniatura z Eksploratora.
- **Historia schowka**: `cb` w palecie (+ tekst do wyszukania). Enter wkleja w okno, w którym byłeś; Tab: przypnij, usuń. Hasła z menedżerów haseł są pomijane.
- **Zrzut z adnotacją**: `Ctrl+Alt+P` albo `zr` — zaznacz obszar (klik = cały ekran), dorysuj strzałkę / ramkę / pisak / zakreślacz, Enter zapisuje do *Obrazy\Screenshots*, kopiuje obraz do schowka i kładzie plik na półce.

## Okna: strefy i układy

- **Strefy** pod ultrawide (trzy kolumny, szeroki środek, połowy, szeroka lewa, ćwiartki): `Ctrl+Alt+1…6` albo przeciąganie okna z wciśniętym Shift.
- **Układy**: `uz` zapisuje położenie wszystkich okien (domyślnie jako tryb + monitory), `ul` przywraca i uruchamia brakujące aplikacje. Automatyzacja „Układ okien” zrobi to sama, np. po podłączeniu monitora.

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
