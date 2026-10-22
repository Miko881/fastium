# Fastium

<p align="center">
  <img src="assets/icon.ico" width="80" height="80" alt="Fastium Logo">
  <br>
  <b>Lekka, szybka i minimalistyczna przeglądarka dla systemu Windows 10/11</b>
  <br>
  <i>Napisana w języku Rust, oparta na silniku Microsoft Edge WebView2 Evergreen Runtime</i>
</p>

<p align="center">
  <a href="https://github.com/Miko881/fastium/releases/latest"><img src="https://img.shields.io/badge/Wydanie-FastiumSetup.exe-brightgreen?style=flat-square" alt="Pobierz instalator"></a>
  <img src="https://img.shields.io/badge/Platforma-Windows_10_/_11-blue?style=flat-square" alt="Platforma">
  <img src="https://img.shields.io/badge/Język-Rust_/_Win32-orange?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/Licencja-MIT-green?style=flat-square" alt="Licencja">
</p>

---

## 🚀 Pobieranie i instalacja (dla użytkowników)

Najprostszym sposobem na korzystanie z Fastium jest pobranie oficjalnego instalatora:

1. Przejdź do zakładki **[Releases (Wydania)](https://github.com/Miko881/fastium/releases)**.
2. Pobierz plik **`FastiumSetup.exe`**.
3. Uruchom instalator — zainstaluje Fastium na Twoim koncie użytkownika (nie wymaga uprawnień administratora) i utworzy skróty na Pulpicie oraz w Menu Start.

> [!NOTE]
> Fastium korzysta z systemowego środowiska **Microsoft Edge WebView2**, które jest fabrycznie zainstalowane w każdym Windows 11 i zaktualizowanym Windows 10.

---

## 🌟 Główne cechy

- **Błyskawiczne działanie i minimalny rozmiar:**
  - Sam plik wykonywalny waży zaledwie **~1 MB**, a kompletny instalator z ikoną i deinstalatorem tylko **~2.4 MB**.
  - Brak ciężkich frameworków (Electron, Node.js) — okno tworzy natywne `tao`, a interfejs przeglądarki jest bezpośrednio skompilowany w pliku `.exe`.
- **Wbudowane blokowanie reklam i trackerów:**
  - Natywne blokowanie żądań sieciowych (`WebResourceRequested`) oparte o listę domen (`assets/blocklist.txt`), działające także dla Service Workerów przed załadowaniem strony.
  - Możliwość włączenia/wyłączenia ochrony jednym kliknięciem przyciskiem **„Ochrona”**.
- **Własny, czytelny menedżer pobierania:**
  - Zastępuje natywne wyskakujące okienka Edge estetycznym, ciemnym panelem Fastium.
  - Wyświetlanie prędkości transferu w czasie rzeczywistym (`MB/s`) oraz szacowanego czasu do końca (`Pozostało: X s`).
  - Bezpośrednie akcje: **„Otwórz plik”**, **„W folderze”** (zaznacza plik w Eksploratorze Windows) oraz **„Usuń”** (bezpieczne przenoszenie pliku do Kosza Windows przez Win32 API).
  - Skrót do rzeczywistego folderu **„Folder Pobrane”** (respektuje przeniesione foldery pobrań na dyski D:, E: itp.).
  - Dyskretny wskaźnik aktywnego pobierania na górnym pasku narzędzi.
- **Inteligentne oszczędzanie pamięci RAM:**
  - Ukryte i nieużywane karty po 45 sekundach przechodzą w stan uśpienia (`TrySuspend`).
  - Karty odtwarzające dźwięk (np. muzyka w tle), pobierające pliki lub oznaczone przyciskiem **„Nie usypiaj”** pozostają w pełni aktywne.
  - Nowo otwarte puste karty nie uruchamiają od razu silnika renderującego, oszczędzając zasoby.
- **Wygodny Omnibox i skróty:**
  - Inteligentne rozpoznawanie adresów URL, localhosta, domen i zapytań do wyszukiwarki Google.
  - Pełna obsługa natywnych skrótów klawiszowych nawet wtedy, gdy fokus znajduje się wewnątrz strony internetowej.

---

## ⌨️ Skróty klawiszowe

| Skrót | Działanie |
|---|---|
| <kbd>Ctrl</kbd> + <kbd>L</kbd> | Zaznaczenie paska adresu |
| <kbd>Ctrl</kbd> + <kbd>T</kbd> | Otwarcie nowej karty |
| <kbd>Ctrl</kbd> + <kbd>W</kbd> | Zamknięcie aktywnej karty |
| <kbd>Ctrl</kbd> + <kbd>Tab</kbd> | Przełączenie na następną kartę |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>Tab</kbd> | Przełączenie na poprzednią kartę |
| <kbd>Ctrl</kbd> + <kbd>R</kbd> lub <kbd>F5</kbd> | Odświeżenie strony |
| <kbd>Ctrl</kbd> + <kbd>J</kbd> | Otwarcie / zamknięcie panelu pobierania |

---

## 🛠️ Kompilacja i budowanie (dla deweloperów)

### Wymagania wstępne:
1. System operacyjny: **Windows 10 lub Windows 11**.
2. **Visual Studio Build Tools** (kompilator MSVC C++ oraz Windows SDK).
3. **Rust** zainstalowany przez [rustup](https://rustup.rs/) (architektura `x86_64-pc-windows-msvc`).
4. *(Opcjonalnie do budowy instalatora)* **Inno Setup 6** (można zainstalować przez `winget install JRSoftware.InnoSetup`).

### Uruchomienie w trybie deweloperskim:
```powershell
cargo run
```

### Zbudowanie pełnego wydania i instalatora:
W repozytorium znajduje się zautomatyzowany skrypt wydania:
```powershell
.\scripts\build-release.ps1
```

Skrypt automatycznie:
1. Sprawdza poprawność zależności (`Cargo.lock`).
2. Uruchamia testy jednostkowe (`cargo test --locked`).
3. Kompiluje zoptymalizowaną binarkę Release z linkowaniem statycznym CRT i pełnym LTO.
4. Generuje paczkę portable `.zip` z sumami kontrolnymi SHA-256 w katalogu `dist\`.
5. Kompiluje instalator **`FastiumSetup.exe`** w katalogu `dist\`.

---

## 📁 Struktura projektu

```text
Cargo.toml                   Konfiguracja projektu, zależności i profil release
Cargo.lock                   Precyzyjnie przypięte wersje bibliotek
installer.iss                Skrypt instalatora Inno Setup (z detekcją WebView2 i skrótami)
.cargo/config.toml           Konfiguracja statycznego CRT
src/
  ├── main.rs                Punkt wejścia aplikacji i obsługa błędów startowych
  ├── browser.rs             Główna pętla zdarzeń, zarządzanie kartami, usypianie, IPC
  ├── native.rs              Integracja Win32/WebView2: blokowanie, pobieranie, Kosz, foldery
  └── policy.rs              Obsługa adresów omniboksu, filtry blokera reklam i testy
assets/
  ├── shell.html             Lokalny interfejs użytkownika, ciemny motyw i panel pobierania
  ├── blocklist.txt          Wbudowana lista domen blokowanych reklam/trackerów
  ├── icon.ico               Oficjalna ikona aplikacji
  ├── fastium.rc             Zasoby Windows (wbudowanie ikony w plik EXE)
  └── fastium.res            Skompilowane zasoby binarne
scripts/
  ├── build-release.ps1      Główny skrypt budowania wydań i instalatora FastiumSetup.exe
  └── ui-smoke.mjs           Testy integracyjne interfejsu
```

---

## 🔒 Bezpieczeństwo i prywatność

- **Izolacja danych:** Dane przeglądania i ciasteczka stron znajdują się w `%LOCALAPPDATA%\Fastium\Profile`.
- **Ścisłe CSP interfejsu:** Lokalny interfejs przeglądarki (`assets/shell.html`) działa w całkowitej izolacji od stron internetowych (`connect-src 'none'; form-action 'none'`), a komunikacja z backendem w Rust odbywa się wyłącznie za pośrednictwem bezpiecznego IPC.
- **Bezpieczne usuwanie:** Pliki usuwane z poziomu menedżera pobierania są przenoszone do Kosza Windows (`FOF_ALLOWUNDO`), co zapobiega przypadkowej utracie danych.
- **Brak telemetrii:** Fastium nie gromadzi, nie rejestruje ani nie wysyła żadnych danych o odwiedzanych stronach ani zapytaniach.

---

## 📄 Licencja

Projekt objęty jest licencją [MIT](LICENSE).
