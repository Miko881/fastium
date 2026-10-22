# Fastium

Minimalistyczna przeglądarka dla **Windows 10/11**, napisana w Rust. Okno tworzy `tao`, a strony renderuje systemowy **Microsoft Edge WebView2 Evergreen Runtime** przez `wry`. HTML/CSS/JavaScript interfejsu jest osadzony w EXE; nie wymaga Node.js, serwera ani frameworka frontendowego do działania lub kompilacji.

**Status:** implementacja źródłowa 0.1, wymagająca kompilacji i testów integracyjnych na Windows. W środowisku przygotowania projektu nie było toolchainu Rust, a próba jego pobrania zakończyła się blokadą sieci (błąd 10013). Nie deklarujemy zmierzonej wielkości EXE, zużycia RAM ani potwierdzonej poprawności kompilacji. Skrypt wydania wykonuje testy, buduje aplikację i odrzuca EXE oraz ZIP o rozmiarze ≥ 30 000 000 bajtów.

## Co zawiera projekt

- Karty: tworzenie, zamykanie, przełączanie, tytuły, rzeczywisty URL, historia wstecz/dalej, odświeżanie i zatrzymanie ładowania. Nowe puste karty nie tworzą dodatkowych silników WebView.
- Omnibox: HTTP/HTTPS, domeny bez protokołu, localhost z portem, IPv4/IPv6, wyszukiwanie Google z poprawnym kodowaniem zapytania. Schematy `javascript:`, `file:` i `data:` nie są otwierane w kartach.
- Lokalny dashboard Fastium z wyszukiwaniem Google i skrótami.
- Blokowanie żądań reklamowych/trackerów przez `WebResourceRequested`, również dla workerów, przed pierwszą nawigacją. Lista domen w `assets/blocklist.txt`; dopasowanie pełnej domeny lub jej subdomen, bez przypadkowych dopasowań fragmentów nazw. Przycisk „Ochrona” zmienia filtr globalnie.
- Panel pobierania: rzeczywiste bajty z `ICoreWebView2DownloadOperation`, pasek postępu, nieznany rozmiar, pauza, wznowienie, anulowanie i kod przerwania. Pobieranie realizuje WebView2, z jego cookies, TLS, wyborem miejsca zapisu i interfejsem reputacji plików. Fastium nie buforuje pobieranych plików w RAM.
- Karty ukryte dostają `MemoryUsageLevel::Low`; po 45 sekundach następuje `TrySuspend`. Karty z dźwiękiem, aktywnym/wznawialnym pobieraniem lub zaznaczoną opcją „Nie usypiaj” pozostają aktywne. Przełączenie przywraca kartę przez `Resume`, także przy spóźnionym callbacku usypiania.
- Skróty obsługiwane natywnie, również gdy fokus znajduje się wewnątrz strony.

## Kompilacja krok po kroku

1. Zainstaluj **Visual Studio 2022 Build Tools** z workloadem „Desktop development with C++”, MSVC i Windows SDK. Pobranie: [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).
2. Zainstaluj Rust przez [rustup](https://rustup.rs/), wybierając MSVC. W nowym terminalu sprawdź:

   ```powershell
   rustup toolchain install stable --profile minimal --component rustfmt --component clippy
   rustc --version
   cargo --version
   ```

3. Zainstaluj/aktualizuj **WebView2 Evergreen Runtime**: [oficjalna dystrybucja Microsoft](https://developer.microsoft.com/en-us/microsoft-edge/webview2/). Wymagana jest wersja wspierająca `ICoreWebView2_22`, używane do filtrowania również żądań workerów. Przeglądarka Edge i WebView2 Runtime to osobne produkty; obecność Edge nie zastępuje sprawdzenia Runtime.
4. W katalogu projektu uruchom wersję developerską:

   ```powershell
   cd D:\fastium
   cargo run
   ```

5. Zbuduj wydanie:

   ```powershell
   .\scripts\build-release.ps1
   ```

   Skrypt tworzy `Cargo.lock`, jeśli go brakuje, następnie wykonuje `cargo test --locked` i `cargo build --release --locked --target x86_64-pc-windows-msvc`. Plik wykonywalny znajduje się w `target\x86_64-pc-windows-msvc\release\fastium.exe`. Paczka portable, suma SHA-256 i lista zależności trafiają do `dist\`. ZIP nie zawiera silnika Chromium ani WebView2 Runtime.

   Zachowaj wygenerowany `Cargo.lock` w repozytorium. Nie został dostarczony zmyślony lockfile: pierwsze rozwiązanie zależności wymaga połączenia z crates.io. Przypięte wersje bibliotek okienkowych odpowiadają API użytemu w kodzie; nie oznacza to, że są najnowsze. Przed publicznym wydaniem zaktualizuj zależności i przeprowadź testy regresji.

6. Uruchom wydanie:

   ```powershell
   .\target\x86_64-pc-windows-msvc\release\fastium.exe
   ```

Opcjonalna kompilacja ARM64 wymaga toolchainu i bibliotek MSVC dla ARM64:

```powershell
rustup target add aarch64-pc-windows-msvc
.\scripts\build-release.ps1 -Target aarch64-pc-windows-msvc
```

Pipeline `.github/workflows/windows.yml` wykonuje analizę Clippy, testy i kompilację na Windows oraz udostępnia ZIP jako artefakt CI. Samo dodanie pliku workflow nie uruchamia ani nie potwierdza testów.

## Rozmiar i pamięć

Profil release używa `opt-level = "s"`, pełnego LTO, jednej jednostki generowania kodu, usunięcia symboli i `panic = "abort"`. MSVC CRT jest linkowany statycznie. Nie używamy `target-cpu=native`, aby plik mógł działać na starszych procesorach, ani UPX. `opt-level="s"` dotyczy Rustowego hosta; renderer i dekodery WebView2 są oddzielnymi, już skompilowanymi komponentami.

**Limit 30 MB dotyczy Fastium, nie całego środowiska wykonawczego.** WebView2 ma własne pliki, aktualizacje i procesy. Systemowy silnik zmniejsza rozmiar dystrybucji, ale nie usuwa kosztów Chromium w RAM. Runtime pobiera się osobno tylko wtedy, gdy brakuje go na komputerze. Projekt dostarcza paczkę portable, nie instalator offline z Runtime.

Interfejs to jeden lokalny WebView, a każda otwarta strona to osobny WebView we wspólnym profilu. Lokalny interfejs używa oddzielnego profilu. Daje to izolację i prosty UI kosztem dodatkowego renderera. Podział procesów pozostaje decyzją WebView2: jedna karta nie musi oznaczać dokładnie jednego procesu. Zamknięcie karty zwalnia kontroler, lecz Runtime może zachować współdzielone procesy/cache. Usypianie zachowuje DOM i historię; nie obiecuje zerowego RAM.

Pętla zdarzeń używa `WaitUntil`, nie aktywnego `Poll`. Odczyt postępu odbywa się co 500 ms tylko przy istniejących operacjach pobierania, a utrzymanie kart co 2 s. Interfejs otrzymuje stan tylko wtedy, gdy coś się zmieni. Historia pobrań jest ograniczona do 100 wpisów; zakończone operacje zwalniają referencje COM. Próba przekroczenia limitu 100 jednoczesnych operacji zostaje anulowana.

## GPU, YouTube i kodeki

Domyślnie Fastium pozostawia włączoną sprzętową ścieżkę WebView2 oraz automatyczny dobór dekodera. Nie przekazuje `--disable-gpu`, nie wyłącza sandboxa, izolacji stron ani SmartScreen. Jawny pusty zestaw dodatkowych argumentów zastępuje domyślne argumenty `wry 0.48`, które wyłączały m.in. SmartScreen. Zalecenie Microsoft to [niewyłączanie GPU w aplikacjach WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance).

Jeżeli potrzebujesz przetestować wymuszenie ścieżki GPU na sterowniku z listy blokowanych:

```powershell
.\target\x86_64-pc-windows-msvc\release\fastium.exe --force-gpu
```

Ten tryb dodaje `--ignore-gpu-blocklist --enable-gpu-rasterization` i używa osobnego profilu `Fastium\ForceGpu`, bo środowiska z różnymi argumentami nie powinny współdzielić katalogu danych. Może pogorszyć stabilność na wadliwym sterowniku. Powrót do uruchomienia bez tej opcji przywraca profil standardowy. Znaczenie flag i ich dostępność opisuje [dokumentacja WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/webview-features-flags).

**Żadna flaga nie doda dekodera AV1/VP9 do GPU, które go nie posiada.** H.264, VP9 i AV1 są negocjowane przez stronę oraz Runtime; sprzętowe dekodowanie zależy od GPU, sterownika, profilu kodeka i rozdzielczości. Fastium nie podmienia `canPlayType`, nie ukrywa kodeków skryptem i nie obiecuje 100% kompatybilności witryn, DRM czy logowania w osadzonym silniku.

W YouTube otwórz „Statystyki dla nerdów”: porównaj kodek (`avc1`, `vp09`, `av01`), dropped frames i rozdzielczość. W Menedżerze zadań sprawdź silnik GPU „Video Decode” dla procesów `msedgewebview2.exe`. Sam niski CPU nie potwierdza dekodowania sprzętowego. Na starszym GPU przetestuj niższą rozdzielczość i ograniczenie preferowania AV1 w ustawieniach odtwarzania YouTube, jeśli ta opcja jest dostępna.

## Skróty

| Skrót | Działanie |
|---|---|
| Ctrl+L | Fokus i zaznaczenie adresu |
| Ctrl+T | Nowa karta |
| Ctrl+W | Zamknij aktywną kartę |
| Ctrl+Tab / Ctrl+Shift+Tab | Następna / poprzednia karta |
| Ctrl+R lub F5 | Odśwież |
| Ctrl+J | Panel pobierania |

## Granice wersji 0.1

- Blokowanie domen jest lekką listą startową, nie pełnym EasyList/uBlock Origin. Nie usuwa reklam dostarczanych z domen treści, w tym części reklam YouTube. Brak filtrowania kosmetycznego i aktualizacji list online. Zmiana `blocklist.txt` wymaga ponownej kompilacji. Wyłącz „Ochronę” i odśwież stronę, jeśli filtr ją uszkadza.
- Nieaktywny formularz, cichy WebRTC lub obliczenia w tle mogą zostać uśpione. Przycisk „Nie usypiaj” pozwala zachować ich działanie. Nie ma automatycznego odrzucania DOM, aby nie tracić formularzy.
- Panel pokazuje pobrania z bieżącej sesji. Brak wznowienia po restarcie. Zamknięcie aplikacji anuluje pobieranie; zamknięcie karty lub przejście na lokalny dashboard podczas jej pobierania jest odrzucane z komunikatem. Lokalny dashboard resetuje historię tej karty.
- `window.open` otwiera URL w nowej karcie. Nie zachowuje relacji `window.opener`, dlatego niektóre logowania korzystające z popupów mogą wymagać dodatkowej obsługi. Brak menedżera haseł, rozszerzeń, synchronizacji, zakładek i odtwarzania sesji po restarcie.
- Implementacja COM jest specyficzna dla Windows. Porty macOS/Linux wymagają backendów blokowania, pobierania i usypiania; samo użycie `wry` nie zapewnia tych funkcji identycznie na każdej platformie.

## Dane i granica zaufania

Cookies, cache i dane stron: `%LOCALAPPDATA%\Fastium\Profile`. UI: `%LOCALAPPDATA%\Fastium\Interface`. Tryb wymuszenia GPU: podfolder `ForceGpu`. Nie ma telemetrii Fastium; wyszukiwanie trafia do Google dopiero po zatwierdzeniu. Dane profilu nie są usuwane przy zamknięciu.

Polecenia aplikacji odbiera wyłącznie lokalny UI (`about:blank`), który nie może nawigować do sieci i ma CSP blokujące połączenia, ramki i formularze. Zdalne WebView mają wyłączone WebMessage i host objects. Tytuły, URL-e oraz nazwy plików są serializowane przez `serde_json` i wyświetlane przez `textContent`. Pobrania nie są automatycznie uruchamiane. Zdarzenia COM kierują zmiany kart do pętli `tao`, aby uniknąć reentrantnego tworzenia i niszczenia kontrolerów. Wszystkie interfejsy COM pozostają na wątku UI.

## Testy

```powershell
cargo test
cargo clippy --all-targets
cargo fmt --all
# Opcjonalnie, przy Node.js >= 22 i zainstalowanym Edge/Chrome:
node scripts/ui-smoke.mjs
```

Testy Rust w `policy.rs` sprawdzają klasyfikację adresów, kodowanie zapytań, odrzucanie uprzywilejowanych schematów i granice domen blokera. Test UI używa prawdziwego Chromium przez CDP pipe, bez npm; sprawdza renderowanie, IPC, wstrzyknięcie HTML w tytuł/nazwę pliku, edycję omniboksu podczas aktualizacji, przełączanie kart i stan postępu pobierania. Nie testuje natywnej integracji WebView2. Zrzut dashboardu zapisuje w `.tools/ui-test/dashboard.png`. Zmienna `FASTIUM_TEST_BROWSER` pozwala wskazać plik przeglądarki; `FASTIUM_TEST_SOFTWARE=1` używa renderowania programowego tylko w teście UI.

Przed wydaniem sprawdź na docelowym Windows:

1. Nowa karta, adres HTTPS, localhost z portem, wyszukiwanie z polskimi znakami; przekierowania, historia i SPA aktualizują pasek adresu.
2. 10 kart, przełączanie, zamykanie i skalowanie DPI 100/150/200%; górna część strony nie przykrywa paska adresowego, panel pobrań zmniejsza obszar treści.
3. Po 45 s karta bez audio zasypia; po powrocie działa. Karta z YouTube grającym w tle oraz „Nie usypiaj” pozostają aktywne. Wielokrotne szybkie przełączanie w momencie usypiania nie blokuje aktywnej karty.
4. W buildzie debug, w narzędziach deweloperskich strony sprawdź żądanie do domeny z listy: odpowiedź 403 „Blocked by Fastium”. Po wyłączeniu ochrony nowe żądanie nie jest filtrowane. Sprawdź także worker.
5. Duży plik: postęp, pauza, wznowienie, anulowanie; transfer chunked bez Content-Length; błąd sieci; plik istniejący już w docelowym katalogu. Pozostaw natywny UI WebView2 do obsługi komunikatów reputacji.
6. YouTube 1080p60/4K: dropped frames, CPU i GPU Video Decode, tryb domyślny i opcjonalnie wymuszony. Sprawdź również maszynę bez sprzętowego AV1.
7. Zmierz całe drzewo procesów Fastium + WebView2 po 30 s i po 2 min, z 1/10 kartami. Zapisz wersję Runtime, systemu, CPU/GPU i sterownika; porównuj te same strony. Nie porównuj wyłącznie RAM `fastium.exe`.

## Pliki

```text
Cargo.toml                   zależności i profil release
.cargo/config.toml           statyczny CRT
src/main.rs                  wejście i dialog błędu uruchomienia
src/browser.rs               pętla zdarzeń, karty, stan UI, usypianie
src/native.rs                WebView2: filtry, pobrania, skróty, COM
src/policy.rs                omnibox, dopasowanie domen, testy
assets/shell.html            kompletny lokalny interfejs i dashboard
assets/blocklist.txt         wbudowana lista reklam/trackerów
scripts/build-release.ps1    testy, kompilacja, limit rozmiaru, ZIP
scripts/ui-smoke.mjs          test UI bez zależności npm
.github/workflows/windows.yml kompilacja w CI
```
