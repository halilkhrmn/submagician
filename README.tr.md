# SubMagician

Bir klasördeki tüm videolar için altyazı bulur, **senin** dosyan için hazırlanmış olanı seçer,
kodlamasını ve zamanlamasını düzeltir ve videonun yanına kaydeder. Windows ve Linux (macOS sonra).

[English](README.md)

![SubMagician](docs/img/main.png)

## Neden

- Hash eşleşmesi her zaman doğru değil, isimle arayınca da onlarca sürüm arasından tahmin
  etmek gerekiyor. SubMagician her adayı puanlar: hash eşleşmesi, release grubu, kaynak
  (BluRay/WEB), yayın servisi, çözünürlük, isim benzerliği; yanlış bölümler elenir.
- Bozuk Türkçe karakterler (Windows-1254) düzeltilir; her şey UTF-8 olarak kaydedilir.
- Zamanlama videonun sesinden düzeltilir: kayma, kare hızı (23.976 / 24 / 25) ve kesilmiş ya
  da eklenmiş sahneler. Altyazı zaten uyuyorsa dokunulmaz. Senkron olan başka bir altyazıya göre
  de senkronlayabilir ya da ±0,1 s / ±1 s kaydırabilirsin.
- Kaynaklar: OpenSubtitles, SubDL ve Addic7ed (diziler, Gestdown üzerinden); her biri
  kapatılabilir. Sonuçlar birkaç gün önbellekte tutulur. RAR ve 7z arşivleri bsdtar / 7-Zip ile
  açılır (Windows 10+ içinde `tar.exe` hazır gelir).

## Kullanım

1. **Choose folder…**, pencereye bir klasör bırak ya da dosya yöneticisinde klasöre sağ tıkla →
   *Find subtitles with SubMagician* (bu girdiyi Settings → File manager ekler).
2. **Download best for all** ya da bir videoya tıklayıp puanlı adaylardan birini indir. Adı
   bozuk bir dosyayı **Search as…** ile başka bir adla arayabilirsin.
3. Altyazı `Film.mkv` dosyasının yanına `Film.tr.srt` olarak kaydedilir; oynatıcılar kendisi
   açar. Üzerine yazılan dosya saklanır, **Restore previous** onu geri getirir.
4. **ffmpeg** varsa indirmeden hemen sonra sese göre senkronlanır; zaten olan bir altyazı için
   **Sync to audio**'ya tıkla. Windows'ta Settings → Timing → **Download ffmpeg** onu indirir.

![Senkron](docs/img/sync.png)

Ayrıca:

- İçinde istenen dilde altyazı izi olan videolar (MKV) atlanır.
- **Watch folder**: klasöre eklenen yeni videolar, kopyalanması bitince altyazısını kendisi alır.
- **Only missing** işi bitmiş videoları gizler; listenin altında **Play** ve **Show in folder**
  var.
- **Write from audio** (Whisper, bu bilgisayarda): hiçbir kaynakta altyazı yoksa konuşmadan
  altyazı yazar. Modeli Settings → Speech'ten seçip indir; istersen otomatik de çalışır.
  İngilizceye her dilden çevirebilir; diğer dillerde konuşulan dilde yazar.

Ayarlar: istenen diller sırayla (`tr, en`), kaynaklar, daha yüksek günlük indirme sınırı için
isteğe bağlı OpenSubtitles girişi, ffmpeg yolu.

## Komut satırı

`submagician-cli` aynısını betikler için yapar, uygulamanın ayarlarını kullanır:

```sh
submagician-cli ~/Videolar                    # her video için en iyi altyazı + senkron
submagician-cli -l tr,en --dry-run Film.mkv   # neyi seçeceğini göster
submagician-cli --sources addic7ed --no-sync ~/Diziler/The.Office
submagician-cli --download-model base && submagician-cli --generate ~/Videolar
```

`submagician <klasör ya da video>` uygulamayı o klasörle açar.

## Derleme

Rust 1.88+, bir C/C++ derleyicisi ve CMake. Linux'ta: `libfontconfig1-dev libxkbcommon-dev`. Senkron ve
gömülü izler için: PATH'te ya da programın yanında `ffmpeg` ve `ffprobe`.

```sh
SUBMAGICIAN_OPENSUBTITLES_API_KEY=… SUBMAGICIAN_SUBDL_API_KEY=… \
  cargo build --release -p submagician -p submagician-cli
```

Yol haritası için `docs/PLAN.md`. Lisans: AGPL-3.0.
