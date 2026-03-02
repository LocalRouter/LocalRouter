# Third-Party Licenses

LocalRouter is licensed under AGPL-3.0-or-later. This file documents the open source
licenses of third-party dependencies used in this project.

All third-party dependencies use permissive or weak-copyleft licenses that are compatible
with AGPL-3.0-or-later distribution.

## License Categories

| License | Type | Count |
|---------|------|-------|
| MIT | Permissive | Majority |
| Apache-2.0 | Permissive | Many (often dual MIT/Apache-2.0) |
| ISC | Permissive | Several |
| BSD-2-Clause | Permissive | Few |
| BSD-3-Clause | Permissive | Few |
| MPL-2.0 | Weak copyleft | Few (file-level) |
| Unicode-3.0 | Permissive | ICU4X components |
| Unlicense | Public domain equiv. | Few |
| Zlib | Permissive | Few |
| CC0-1.0 | Public domain | notify |
| CC-BY-4.0 | Permissive | caniuse-lite (data) |
| BSL-1.0 | Permissive | clipboard-win, ryu |
| 0BSD | Permissive | tslib, adler2 |

---

## Rust Dependencies (Backend)

| Crate | Version | License | Repository |
|-------|---------|---------|------------|
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | https://github.com/oyvindln/adler2 |
| ahash | 0.8.12 | MIT OR Apache-2.0 | https://github.com/tkaitchuck/ahash |
| aho-corasick | 1.1.4 | Unlicense OR MIT | https://github.com/BurntSushi/aho-corasick |
| alloc-no-stdlib | 2.0.4 | BSD-3-Clause | https://github.com/dropbox/rust-alloc-no-stdlib |
| alloc-stdlib | 0.2.2 | BSD-3-Clause | https://github.com/dropbox/rust-alloc-no-stdlib |
| android_system_properties | 0.1.5 | MIT OR Apache-2.0 | https://github.com/nical/android_system_properties |
| anstream | 0.6.21 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle | 1.0.13 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle-parse | 0.2.7 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle-query | 1.1.5 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle-wincon | 3.0.11 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anyhow | 1.0.100 | MIT OR Apache-2.0 | https://github.com/dtolnay/anyhow |
| arbitrary | 1.4.2 | MIT OR Apache-2.0 | https://github.com/rust-fuzz/arbitrary/ |
| arboard | 3.6.1 | MIT OR Apache-2.0 | https://github.com/1Password/arboard |
| assert-json-diff | 2.0.2 | MIT | https://github.com/davidpdrsn/assert-json-diff.git |
| async-stream | 0.3.6 | MIT | https://github.com/tokio-rs/async-stream |
| async-trait | 0.1.89 | MIT OR Apache-2.0 | https://github.com/dtolnay/async-trait |
| atk | 0.18.2 | MIT | https://github.com/gtk-rs/gtk3-rs |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT | https://github.com/smol-rs/atomic-waker |
| autocfg | 1.5.0 | Apache-2.0 OR MIT | https://github.com/cuviper/autocfg |
| axum | 0.7.9 | MIT | https://github.com/tokio-rs/axum |
| axum-core | 0.4.5 | MIT | https://github.com/tokio-rs/axum |
| axum-macros | 0.4.2 | MIT | https://github.com/tokio-rs/axum |
| base64 | 0.13.1 | MIT OR Apache-2.0 | https://github.com/marshallpierce/rust-base64 |
| bcrypt | 0.16.0 | MIT | https://github.com/Keats/rust-bcrypt |
| bit-set | 0.5.3 | MIT OR Apache-2.0 | https://github.com/contain-rs/bit-set |
| bit-vec | 0.6.3 | MIT OR Apache-2.0 | https://github.com/contain-rs/bit-vec |
| bitflags | 1.3.2 | MIT OR Apache-2.0 | https://github.com/bitflags/bitflags |
| block | 0.1.6 | MIT | http://github.com/SSheldon/rust-block |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| block2 | 0.6.2 | MIT | https://github.com/madsmtm/objc2 |
| blowfish | 0.9.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/block-ciphers |
| brotli | 8.0.2 | BSD-3-Clause AND MIT | https://github.com/dropbox/rust-brotli |
| brotli-decompressor | 5.0.0 | BSD-3-Clause OR MIT | https://github.com/dropbox/rust-brotli-decompressor |
| bumpalo | 3.19.1 | MIT OR Apache-2.0 | https://github.com/fitzgen/bumpalo |
| bytecount | 0.6.9 | Apache-2.0 OR MIT | https://github.com/llogiq/bytecount |
| bytemuck | 1.24.0 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| byteorder | 1.5.0 | Unlicense OR MIT | https://github.com/BurntSushi/byteorder |
| byteorder-lite | 0.1.0 | Unlicense OR MIT | https://github.com/image-rs/byteorder-lite |
| bytes | 1.11.0 | MIT | https://github.com/tokio-rs/bytes |
| cairo-rs | 0.18.5 | MIT | https://github.com/gtk-rs/gtk-rs-core |
| camino | 1.2.2 | MIT OR Apache-2.0 | https://github.com/camino-rs/camino |
| candle-core | 0.8.4 | MIT OR Apache-2.0 | https://github.com/huggingface/candle |
| candle-metal-kernels | 0.8.4 | MIT OR Apache-2.0 | https://github.com/huggingface/candle |
| candle-nn | 0.8.4 | MIT OR Apache-2.0 | https://github.com/huggingface/candle |
| candle-transformers | 0.8.4 | MIT OR Apache-2.0 | https://github.com/huggingface/candle |
| cargo-platform | 0.1.9 | MIT OR Apache-2.0 | https://github.com/rust-lang/cargo |
| cargo_metadata | 0.19.2 | MIT | https://github.com/oli-obk/cargo_metadata |
| cargo_toml | 0.22.3 | Apache-2.0 OR MIT | https://gitlab.com/lib.rs/cargo_toml |
| castaway | 0.2.4 | MIT | https://github.com/sagebind/castaway |
| cc | 1.2.52 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| cesu8 | 1.1.0 | Apache-2.0 OR MIT | https://github.com/emk/cesu8-rs |
| cfb | 0.7.3 | MIT | https://github.com/mdsteele/rust-cfb |
| cfg-expr | 0.15.8 | MIT OR Apache-2.0 | https://github.com/EmbarkStudios/cfg-expr |
| cfg-if | 1.0.4 | MIT OR Apache-2.0 | https://github.com/rust-lang/cfg-if |
| cfg_aliases | 0.2.1 | MIT | https://github.com/katharostech/cfg_aliases |
| chrono | 0.4.42 | MIT OR Apache-2.0 | https://github.com/chronotope/chrono |
| cipher | 0.4.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| clap | 4.5.54 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap |
| clipboard-win | 5.4.1 | BSL-1.0 | https://github.com/DoumanAsh/clipboard-win |
| colorchoice | 1.0.4 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| combine | 4.6.7 | MIT | https://github.com/Marwes/combine |
| command-group | 5.0.1 | Apache-2.0 OR MIT | https://github.com/watchexec/command-group |
| compact_str | 0.9.0 | MIT | https://github.com/ParkMyCar/compact_str |
| console | 0.15.11 | MIT | https://github.com/console-rs/console |
| convert_case | 0.4.0 | MIT | https://github.com/rutrum/convert-case |
| cookie | 0.18.1 | MIT OR Apache-2.0 | https://github.com/SergioBenitez/cookie-rs |
| core-foundation | 0.9.4 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| crc32fast | 1.5.0 | MIT OR Apache-2.0 | https://github.com/srijs/rust-crc32fast |
| crossbeam-channel | 0.5.15 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| cssparser | 0.29.6 | MPL-2.0 | https://github.com/servo/rust-cssparser |
| csv | 1.4.0 | Unlicense OR MIT | https://github.com/BurntSushi/rust-csv |
| darling | 0.20.11 | MIT | https://github.com/TedDriggs/darling |
| dashmap | 6.1.0 | MIT | https://github.com/xacrimon/dashmap |
| deadpool | 0.12.3 | MIT OR Apache-2.0 | https://github.com/bikeshedder/deadpool |
| deranged | 0.5.5 | MIT OR Apache-2.0 | https://github.com/jhpratt/deranged |
| derive_builder | 0.20.2 | MIT OR Apache-2.0 | https://github.com/colin-kiegel/rust-derive-builder |
| derive_more | 0.99.20 | MIT | https://github.com/JelteF/derive_more |
| digest | 0.10.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| dirs | 5.0.1 | MIT OR Apache-2.0 | https://github.com/soc/dirs-rs |
| dispatch | 0.2.0 | MIT | http://github.com/SSheldon/rust-dispatch |
| dlopen2 | 0.8.2 | MIT | https://github.com/OpenByteDev/dlopen2 |
| dtoa-short | 0.3.5 | MPL-2.0 | https://github.com/upsuper/dtoa-short |
| dunce | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 | https://gitlab.com/kornelski/dunce |
| dyn-clone | 1.0.20 | MIT OR Apache-2.0 | https://github.com/dtolnay/dyn-clone |
| either | 1.15.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/either |
| encoding_rs | 0.8.35 | (Apache-2.0 OR MIT) AND BSD-3-Clause | https://github.com/hsivonen/encoding_rs |
| erased-serde | 0.4.9 | MIT OR Apache-2.0 | https://github.com/dtolnay/erased-serde |
| error-code | 3.3.2 | BSL-1.0 | https://github.com/DoumanAsh/error-code |
| esaxx-rs | 0.1.10 | Apache-2.0 | https://github.com/Narsil/esaxx-rs |
| fancy-regex | 0.13.0 | MIT | https://github.com/fancy-regex/fancy-regex |
| fastrand | 2.3.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/fastrand |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 | https://github.com/image-rs/fdeflate |
| flate2 | 1.1.8 | MIT OR Apache-2.0 | https://github.com/rust-lang/flate2-rs |
| fnv | 1.0.7 | Apache-2.0 OR MIT | https://github.com/servo/rust-fnv |
| foreign-types | 0.3.2 | MIT OR Apache-2.0 | https://github.com/sfackler/foreign-types |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| fraction | 0.15.3 | MIT OR Apache-2.0 | https://github.com/dnsl48/fraction.git |
| fsevent-sys | 4.1.0 | MIT | https://github.com/octplane/fsevent-rust |
| futures | 0.3.31 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| gemm | 0.17.1 | MIT | https://github.com/sarah-ek/gemm/ |
| gethostname | 1.1.0 | Apache-2.0 | https://codeberg.org/swsnr/gethostname.rs.git |
| getrandom | 0.1.16 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| glib | 0.18.5 | MIT | https://github.com/gtk-rs/gtk-rs-core |
| glob | 0.3.3 | MIT OR Apache-2.0 | https://github.com/rust-lang/glob |
| gtk | 0.18.2 | MIT | https://github.com/gtk-rs/gtk3-rs |
| h2 | 0.3.27 | MIT | https://github.com/hyperium/h2 |
| half | 2.7.1 | MIT OR Apache-2.0 | https://github.com/VoidStarKat/half-rs |
| hashbrown | 0.12.3 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| heck | 0.4.1 | MIT OR Apache-2.0 | https://github.com/withoutboats/heck |
| hex | 0.4.3 | MIT OR Apache-2.0 | https://github.com/KokaKiwi/rust-hex |
| hf-hub | 0.4.3 | Apache-2.0 | https://github.com/huggingface/hf-hub |
| html5ever | 0.29.1 | MIT OR Apache-2.0 | https://github.com/servo/html5ever |
| http | 0.2.12 | MIT OR Apache-2.0 | https://github.com/hyperium/http |
| http-body | 0.4.6 | MIT | https://github.com/hyperium/http-body |
| http-body-util | 0.1.3 | MIT | https://github.com/hyperium/http-body |
| httparse | 1.10.1 | MIT OR Apache-2.0 | https://github.com/seanmonstar/httparse |
| hyper | 0.14.32 | MIT | https://github.com/hyperium/hyper |
| hyper-rustls | 0.24.2 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/hyper-rustls |
| hyper-tls | 0.6.0 | MIT OR Apache-2.0 | https://github.com/hyperium/hyper-tls |
| hyper-util | 0.1.19 | MIT | https://github.com/hyperium/hyper-util |
| iana-time-zone | 0.1.64 | MIT OR Apache-2.0 | https://github.com/strawlab/iana-time-zone |
| ico | 0.4.0 | MIT | https://github.com/mdsteele/rust-ico |
| icu_collections | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties | 2.1.2 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_provider | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| idna | 1.1.0 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| if-addrs | 0.13.4 | MIT OR BSD-3-Clause | https://github.com/messense/if-addrs |
| image | 0.25.9 | MIT OR Apache-2.0 | https://github.com/image-rs/image |
| indexmap | 1.9.3 | Apache-2.0 OR MIT | https://github.com/bluss/indexmap |
| indicatif | 0.17.11 | MIT | https://github.com/console-rs/indicatif |
| infer | 0.19.0 | MIT | https://github.com/bojand/infer |
| inotify | 0.9.6 | ISC | https://github.com/hannobraun/inotify |
| ipnet | 2.11.0 | MIT OR Apache-2.0 | https://github.com/krisprice/ipnet |
| itertools | 0.14.0 | MIT OR Apache-2.0 | https://github.com/rust-itertools/itertools |
| itoa | 1.0.17 | MIT OR Apache-2.0 | https://github.com/dtolnay/itoa |
| javascriptcore-rs | 1.1.2 | MIT | https://github.com/tauri-apps/javascriptcore-rs |
| jni | 0.21.1 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-rs |
| json-patch | 3.0.1 | MIT OR Apache-2.0 | https://github.com/idubrov/json-patch |
| jsonschema | 0.18.3 | MIT | https://github.com/Stranger6667/jsonschema-rs |
| keyboard-types | 0.7.0 | MIT OR Apache-2.0 | https://github.com/pyfisch/keyboard-types |
| keyring | 3.6.3 | MIT OR Apache-2.0 | https://github.com/hwchen/keyring-rs.git |
| kqueue | 1.1.1 | MIT | https://gitlab.com/rust-kqueue/rust-kqueue |
| kuchikiki | 0.8.8 | MIT | https://github.com/brave/kuchikiki |
| lazy_static | 1.5.0 | MIT OR Apache-2.0 | https://github.com/rust-lang-nursery/lazy-static.rs |
| libappindicator | 0.9.0 | Apache-2.0 OR MIT | — |
| libc | 0.2.180 | MIT OR Apache-2.0 | https://github.com/rust-lang/libc |
| libloading | 0.7.4 | ISC | https://github.com/nagisa/rust_libloading/ |
| libsqlite3-sys | 0.30.1 | MIT | https://github.com/rusqlite/rusqlite |
| linux-keyutils | 0.2.4 | Apache-2.0 OR MIT | https://github.com/landhb/linux-keyutils |
| lock_api | 0.4.14 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| log | 0.4.29 | MIT OR Apache-2.0 | https://github.com/rust-lang/log |
| machine-uid | 0.5.3 | MIT | — |
| matchit | 0.7.3 | MIT AND BSD-3-Clause | https://github.com/ibraheemdev/matchit |
| memchr | 2.7.6 | Unlicense OR MIT | https://github.com/BurntSushi/memchr |
| memmap2 | 0.9.9 | MIT OR Apache-2.0 | https://github.com/RazrFalcon/memmap2-rs |
| metal | 0.27.0 | MIT OR Apache-2.0 | https://github.com/gfx-rs/metal-rs |
| mime | 0.3.17 | MIT OR Apache-2.0 | https://github.com/hyperium/mime |
| minisign-verify | 0.2.4 | MIT | https://github.com/jedisct1/rust-minisign-verify |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide |
| mio | 0.8.11 | MIT | https://github.com/tokio-rs/mio |
| moxcms | 0.7.11 | BSD-3-Clause OR Apache-2.0 | https://github.com/awxkee/moxcms.git |
| muda | 0.17.1 | Apache-2.0 OR MIT | https://github.com/amrbashir/muda |
| native-tls | 0.2.14 | MIT OR Apache-2.0 | https://github.com/sfackler/rust-native-tls |
| ndk | 0.9.0 | MIT OR Apache-2.0 | https://github.com/rust-mobile/ndk |
| nix | 0.27.1 | MIT | https://github.com/nix-rust/nix |
| nom | 7.1.3 | MIT | https://github.com/Geal/nom |
| notify | 6.1.1 | CC0-1.0 | https://github.com/notify-rs/notify.git |
| num | 0.4.3 | MIT OR Apache-2.0 | https://github.com/rust-num/num |
| oauth2 | 4.4.2 | MIT OR Apache-2.0 | https://github.com/ramosbugs/oauth2-rs |
| objc | 0.2.7 | MIT | http://github.com/SSheldon/rust-objc |
| objc2 | 0.6.3 | MIT | https://github.com/madsmtm/objc2 |
| ollama-rs | 0.2.2 | MIT | — |
| once_cell | 1.21.3 | MIT OR Apache-2.0 | https://github.com/matklad/once_cell |
| onig | 6.5.1 | MIT | https://github.com/iwillspeak/rust-onig |
| open | 5.3.3 | MIT | https://github.com/Byron/open-rs |
| openssl | 0.10.75 | Apache-2.0 | https://github.com/rust-openssl/rust-openssl |
| option-ext | 0.2.0 | MPL-2.0 | https://github.com/soc/option-ext.git |
| os_pipe | 1.2.3 | MIT | https://github.com/oconnor663/os_pipe.rs |
| pango | 0.18.3 | MIT | https://github.com/gtk-rs/gtk-rs-core |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| paste | 1.0.15 | MIT OR Apache-2.0 | https://github.com/dtolnay/paste |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| phf | 0.8.0 | MIT | https://github.com/sfackler/rust-phf |
| pin-project-lite | 0.2.16 | Apache-2.0 OR MIT | https://github.com/taiki-e/pin-project-lite |
| plist | 1.8.0 | MIT | https://github.com/ebarnard/rust-plist/ |
| png | 0.17.16 | MIT OR Apache-2.0 | https://github.com/image-rs/image-png |
| proc-macro2 | 1.0.105 | MIT OR Apache-2.0 | https://github.com/dtolnay/proc-macro2 |
| pulp | 0.18.22 | MIT | https://github.com/sarah-ek/pulp/ |
| quick-xml | 0.38.4 | MIT | https://github.com/tafia/quick-xml |
| quinn | 0.11.9 | MIT OR Apache-2.0 | https://github.com/quinn-rs/quinn |
| quote | 1.0.43 | MIT OR Apache-2.0 | https://github.com/dtolnay/quote |
| rand | 0.7.3 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib | https://github.com/rust-windowing/raw-window-handle |
| rayon | 1.11.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/rayon |
| regex | 1.12.2 | MIT OR Apache-2.0 | https://github.com/rust-lang/regex |
| reqwest | 0.11.27 | MIT OR Apache-2.0 | https://github.com/seanmonstar/reqwest |
| rfd | 0.16.0 | MIT | https://github.com/PolyMeilex/rfd |
| ring | 0.17.14 | Apache-2.0 AND ISC | https://github.com/briansmith/ring |
| rusqlite | 0.32.1 | MIT | https://github.com/rusqlite/rusqlite |
| rustls | 0.21.12 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/rustls |
| rustls-webpki | 0.101.7 | ISC | https://github.com/rustls/webpki |
| ryu | 1.0.22 | Apache-2.0 OR BSL-1.0 | https://github.com/dtolnay/ryu |
| safetensors | 0.4.5 | Apache-2.0 | https://github.com/huggingface/safetensors |
| scc | 2.4.0 | Apache-2.0 | https://github.com/wvwwvwwv/scalable-concurrent-containers/ |
| schemars | 0.8.22 | MIT | https://github.com/GREsau/schemars |
| security-framework | 2.11.1 | MIT OR Apache-2.0 | https://github.com/kornelski/rust-security-framework |
| selectors | 0.24.0 | MPL-2.0 | https://github.com/servo/servo |
| semver | 1.0.27 | MIT OR Apache-2.0 | https://github.com/dtolnay/semver |
| serde | 1.0.228 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_json | 1.0.149 | MIT OR Apache-2.0 | https://github.com/serde-rs/json |
| serde_yaml | 0.9.34 | MIT OR Apache-2.0 | https://github.com/dtolnay/serde-yaml |
| serial_test | 3.3.1 | MIT | https://github.com/palfrey/serial_test/ |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| shared_child | 1.1.1 | MIT | https://github.com/oconnor663/shared_child.rs |
| shell-words | 1.1.1 | MIT OR Apache-2.0 | https://github.com/tmiasko/shell-words |
| signal-hook | 0.3.18 | Apache-2.0 OR MIT | https://github.com/vorner/signal-hook |
| smallvec | 1.15.1 | MIT OR Apache-2.0 | https://github.com/servo/rust-smallvec |
| socket2 | 0.5.10 | MIT OR Apache-2.0 | https://github.com/rust-lang/socket2 |
| softbuffer | 0.4.8 | MIT OR Apache-2.0 | https://github.com/rust-windowing/softbuffer |
| subtle | 2.6.1 | BSD-3-Clause | https://github.com/dalek-cryptography/subtle |
| syn | 1.0.109 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| tao | 0.34.5 | Apache-2.0 | https://github.com/tauri-apps/tao |
| tar | 0.4.44 | MIT OR Apache-2.0 | https://github.com/alexcrichton/tar-rs |
| tauri | 2.9.5 | Apache-2.0 OR MIT | https://github.com/tauri-apps/tauri |
| tauri-build | 2.5.3 | Apache-2.0 OR MIT | https://github.com/tauri-apps/tauri |
| tauri-plugin-dialog | 2.6.0 | Apache-2.0 OR MIT | https://github.com/tauri-apps/plugins-workspace |
| tauri-plugin-shell | 2.3.4 | Apache-2.0 OR MIT | https://github.com/tauri-apps/plugins-workspace |
| tauri-plugin-updater | 2.9.0 | Apache-2.0 OR MIT | https://github.com/tauri-apps/plugins-workspace |
| tempfile | 3.24.0 | MIT OR Apache-2.0 | https://github.com/Stebalien/tempfile |
| thiserror | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| time | 0.3.45 | MIT OR Apache-2.0 | https://github.com/time-rs/time |
| tokenizers | 0.22.2 | Apache-2.0 | https://github.com/huggingface/tokenizers |
| tokio | 1.49.0 | MIT | https://github.com/tokio-rs/tokio |
| tokio-stream | 0.1.18 | MIT | https://github.com/tokio-rs/tokio |
| tokio-tungstenite | 0.21.0 | MIT | https://github.com/snapview/tokio-tungstenite |
| tokio-util | 0.7.18 | MIT | https://github.com/tokio-rs/tokio |
| toml | 0.8.2 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| tower | 0.5.3 | MIT | https://github.com/tower-rs/tower |
| tower-http | 0.6.8 | MIT | https://github.com/tower-rs/tower-http |
| tracing | 0.1.44 | MIT | https://github.com/tokio-rs/tracing |
| tracing-subscriber | 0.3.22 | MIT | https://github.com/tokio-rs/tracing |
| tray-icon | 0.21.3 | MIT OR Apache-2.0 | https://github.com/tauri-apps/tray-icon |
| tungstenite | 0.21.0 | MIT OR Apache-2.0 | https://github.com/snapview/tungstenite-rs |
| unicode-ident | 1.0.22 | (MIT OR Apache-2.0) AND Unicode-3.0 | https://github.com/dtolnay/unicode-ident |
| untrusted | 0.9.0 | ISC | https://github.com/briansmith/untrusted |
| ureq | 2.12.1 | MIT OR Apache-2.0 | https://github.com/algesten/ureq |
| url | 2.5.8 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| urlencoding | 2.1.3 | MIT | https://github.com/kornelski/rust_urlencoding |
| utoipa | 5.4.0 | MIT OR Apache-2.0 | https://github.com/juhaku/utoipa |
| utoipa-axum | 0.2.0 | MIT OR Apache-2.0 | https://github.com/juhaku/utoipa |
| utoipa-scalar | 0.2.0 | MIT OR Apache-2.0 | https://github.com/juhaku/utoipa |
| uuid | 1.19.0 | Apache-2.0 OR MIT | https://github.com/uuid-rs/uuid |
| walkdir | 2.5.0 | Unlicense OR MIT | https://github.com/BurntSushi/walkdir |
| wasm-bindgen | 0.2.106 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen |
| webpki-roots | 0.25.4 | MPL-2.0 | https://github.com/rustls/webpki-roots |
| webkit2gtk | 2.0.1 | MIT | https://github.com/tauri-apps/webkit2gtk-rs |
| which | 7.0.3 | MIT | https://github.com/harryfei/which-rs.git |
| winapi | 0.3.9 | MIT OR Apache-2.0 | https://github.com/retep998/winapi-rs |
| window-vibrancy | 0.6.0 | Apache-2.0 OR MIT | https://github.com/tauri-apps/tauri-plugin-vibrancy |
| windows | 0.61.3 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| wiremock | 0.6.5 | MIT OR Apache-2.0 | https://github.com/LukeMathWalker/wiremock-rs |
| wry | 0.53.5 | Apache-2.0 OR MIT | https://github.com/tauri-apps/wry |
| x11 | 2.21.0 | MIT | https://github.com/AltF02/x11-rs.git |
| x11rb | 0.13.2 | MIT OR Apache-2.0 | https://github.com/psychon/x11rb |
| zerocopy | 0.8.33 | BSD-2-Clause OR Apache-2.0 OR MIT | https://github.com/google/zerocopy |
| zeroize | 1.8.2 | Apache-2.0 OR MIT | https://github.com/RustCrypto/utils |
| zip | 1.1.4 | MIT | https://github.com/zip-rs/zip2.git |
| zopfli | 0.8.3 | Apache-2.0 | https://github.com/zopfli-rs/zopfli |

## Frontend Dependencies (Website)

| Package | Version | License |
|---------|---------|---------|
| @dnd-kit/core | 6.3.1 | MIT |
| @dnd-kit/sortable | 10.0.0 | MIT |
| @dnd-kit/utilities | 3.2.2 | MIT |
| @heroicons/react | 2.2.0 | MIT |
| @radix-ui/react-alert-dialog | 1.1.15 | MIT |
| @radix-ui/react-checkbox | 1.3.3 | MIT |
| @radix-ui/react-collapsible | 1.1.12 | MIT |
| @radix-ui/react-dialog | 1.1.15 | MIT |
| @radix-ui/react-dropdown-menu | 2.1.16 | MIT |
| @radix-ui/react-hover-card | 1.1.15 | MIT |
| @radix-ui/react-label | 2.1.8 | MIT |
| @radix-ui/react-popover | 1.1.15 | MIT |
| @radix-ui/react-progress | 1.1.8 | MIT |
| @radix-ui/react-radio-group | 1.3.8 | MIT |
| @radix-ui/react-scroll-area | 1.2.10 | MIT |
| @radix-ui/react-select | 2.2.6 | MIT |
| @radix-ui/react-separator | 1.1.8 | MIT |
| @radix-ui/react-slider | 1.3.6 | MIT |
| @radix-ui/react-slot | 1.2.4 | MIT |
| @radix-ui/react-switch | 1.2.6 | MIT |
| @radix-ui/react-tabs | 1.1.13 | MIT |
| @radix-ui/react-tooltip | 1.2.8 | MIT |
| @tanstack/react-table | 8.21.3 | MIT |
| @tauri-apps/api | 2.10.1 | Apache-2.0 OR MIT |
| autoprefixer | 10.4.23 | MIT |
| caniuse-lite | 1.0.30001764 | CC-BY-4.0 |
| class-variance-authority | 0.7.1 | Apache-2.0 |
| clsx | 2.1.1 | MIT |
| cmdk | 1.1.1 | MIT |
| dagre | 0.8.5 | MIT |
| graphlib | 2.1.8 | MIT |
| immer | 10.2.0 | MIT |
| lucide-react | 0.562.0 | ISC |
| postcss | 8.5.6 | MIT |
| react | 18.3.1 | MIT |
| react-dom | 18.3.1 | MIT |
| react-markdown | 10.1.0 | MIT |
| react-resizable-panels | 4.5.9 | MIT |
| react-router-dom | 7.12.0 | MIT |
| reactflow | 11.11.4 | MIT |
| recharts | 3.7.0 | MIT |
| redux | 5.0.1 | MIT |
| remark-gfm | 4.0.1 | MIT |
| sonner | 2.0.7 | MIT |
| source-map-js | 1.2.1 | BSD-3-Clause |
| tailwind-merge | 3.4.0 | MIT |
| tailwindcss | 3.4.19 | MIT |
| tailwindcss-animate | 1.0.7 | MIT |
| ts-interface-checker | 0.1.13 | Apache-2.0 |
| typescript | 5.9.3 | Apache-2.0 |
| vite | 6.4.1 | MIT |
| zustand | 4.5.7 | MIT |

## Notes

- **MPL-2.0** (`cssparser`, `dtoa-short`, `option-ext`, `selectors`, `webpki-roots`): Weak copyleft at the file level. Compatible with AGPL-3.0 and does not affect the licensing of other files.
- **Unicode-3.0** (ICU4X: `icu_collections`, `icu_normalizer`, `icu_properties`, etc.): Permissive license allowing free use with attribution.
- **ISC** (`ring`, `rustls-webpki`, `untrusted`, `inotify`, `libloading`, `lucide-react`, d3 packages): Permissive, functionally equivalent to MIT.
- **CC-BY-4.0** (`caniuse-lite`): Creative Commons Attribution 4.0 — requires attribution for the data.
- **BSL-1.0** (`clipboard-win`, `error-code`, `ryu`): Permissive, similar to MIT.
- **[BloopAI/vibe-kanban](https://github.com/BloopAI/vibe-kanban)** (Apache-2.0): The coding agents orchestration feature was inspired by vibe-kanban's approach to agent process management. No code was directly copied; the implementation uses `command-group` for process management instead.
