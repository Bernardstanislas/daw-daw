# daw-daw

Un petit DAW de bureau (Tauri v2 + Rust + React), inspiré de l'esprit de Cakewalk Next : une
seule fenêtre, une arrangement claire avec des **sections** (Intro / Couplet / Refrain…) qu'on
réordonne en les glissant, un piano roll, un mixer, et des instruments/effets intégrés.
Tout le code, les sons (100 % synthétisés) et les presets sont originaux.

## Lancer

```bash
pnpm install
pnpm tauri dev        # app de bureau avec le moteur audio Rust
pnpm dev:web          # version navigateur : même moteur Rust compilé en WebAssembly (son + MIDI)
pnpm tauri build      # .app / .dmg dans src-tauri/target/release/bundle
```

Version navigateur (`pnpm dev:web`, puis http://localhost:1420 dans Chrome ou Edge) :
le moteur tourne dans un AudioWorklet, avec les mêmes instruments, effets et samples que l'app
de bureau. Les claviers MIDI branchés sont détectés via Web MIDI (Chrome demande l'autorisation).
Le musical typing marche aussi. On importe de l'audio en le déposant sur l'arrangement ou avec
« Import audio… » dans l'onglet Files, et l'export WAV se télécharge. Le son démarre au premier
clic ou à la première touche, c'est une règle des navigateurs. Pas disponible dans le navigateur :
l'enregistrement micro, la navigation dans les dossiers et la sauvegarde avec les fichiers audio.
Le projet se télécharge en JSON, et les audios importés sont à réimporter après un rechargement.

Tests :

```bash
cd crates/daw-core && cargo test     # moteur, DSP (instruments/effets), sauvegarde
cd crates/daw-wasm && cargo test     # API WebAssembly
npx tsc --noEmit                      # typage du frontend
```

Au premier lancement, la démo « Night Drive » est chargée. Ensuite la dernière session est
restaurée automatiquement (autosave local toutes les 4 s) ; utilisez ⌘S pour sauvegarder un vrai
projet.

## Fonctionnalités

- **Pages** : trois onglets en bas de la fenêtre, dans l'ordre du travail, sur le même projet et le
  même transport (touches 1 / 2 / 3).
  - *Arrange* : la timeline, les sections et l'éditeur de clip (piano roll, pas à pas). Les réglages
    du clip ouvert (nom, position, boucle, gain, fondus) sont à gauche de l'éditeur.
  - *Sound* : c'est ici, et seulement ici, qu'on règle les instruments et les effets. Une piste à
    la fois, son instrument et sa chaîne d'effets étalés sur toute la fenêtre, avec un clavier (ou des pads pour les batteries) pour jouer le son. La chaîne du master
    s'y règle aussi.
  - *Mix* : la console pleine hauteur, avec les inserts de chaque tranche (clic = bypass,
    « + » = ajouter, glisser un effet depuis le navigateur). Un double-clic sur le nom d'une tranche
    ouvre la piste dans Sound.
- **Transport** : lecture/stop, boucle (bande haute de la règle), métronome + décompte (clic droit
  sur le métronome : 0/1/2 mesures), tempo, signature, enregistrement.
- **Arrangement** : pistes instrument / audio, clips MIDI et audio, déplacement (⌥ = copie,
  ⌘ = sans grille), redimensionnement, trim, split (S), duplication (⌘D), copier/coller,
  sélection au lasso, mute de clip, couleurs, zoom (⌘ + molette), suivi de la tête de lecture.
- **Arranger** : dessinez des sections dans la ligne « Arranger » ; glissez une section pour
  réordonner tout le morceau (les clips sont découpés et suivent), dupliquez/supprimez une
  section avec son contenu, bouclez une section.
- **Piano roll** : crayon / sélection, grille, quantize (Q), transposition (↑↓, ⇧ = octave),
  vélocité, notes fantômes des clips en boucle, vue « Fold » (lignes nommées pour la batterie).
- **Instruments** :
  - *Sampler* : 22 sons samplés (Grand Piano, Upright Piano, Tine EP, Retro EP, Strings, Violins,
    Viola, Cellos, Brass, Horns, Clarinet, Flute, Harp, Vibraphone, guitares acoustique et
    électrique, orgues, Vox Ahh, Humming…), avec boucle de sustain à fondu, ADSR, transposition et
    tonalité. Il y a un preset par son, avec son enveloppe d'origine.
  - *Sample Kit* : batterie samplée TightKit (kick, snare, clap, charleston fermé et ouvert avec
    étouffement, tom, conga, shaker), mappée GM.
  - *Pulse Synth* : soustractif 16 voix, 2 oscillateurs + sub + bruit, unisson, filtre ZDF,
    2 enveloppes, LFO synchronisable, glide/mono.
  - *Drum Kit* : batterie synthétisée, 4 caractères.
  - *Keys* : piano électrique FM, orgue à tirettes, piano doux.
- **Samples** : les WAV du Sampler et du kit sont embarqués dans l'app (`crates/daw-core/samples/`,
  provenance dans `NOTICE.md`). Le bouton « Library » de l'onglet Files les dépose dans
  `~/Music/daw-daw/Library` pour les pré-écouter et les glisser comme clips audio.
- **Plugins tiers (app de bureau, macOS)** : tous les Audio Units installés (Vital, Surge XT,
  Zebra, Serum…) apparaissent dans le groupe « Plugins » des onglets Instruments et Effects.
  Un clic sur un instrument le pré-écoute au clavier MIDI, un glisser ou un double-clic l'ajoute.
  Sur la page Sound, « Open editor » ouvre le plugin dans sa propre fenêtre (redimensionnable,
  plein écran avec le bouton vert). **Ctrl+Tab** bascule entre la fenêtre principale et les plugins.
  Son état (ton patch) est sauvegardé dans le projet, et le plugin reçoit le tempo et le transport
  (LFO synchronisés).
  Les plugins sont aussi rendus dans l'export WAV.
- **Effets** : Channel EQ, Compressor, Echo, Space (reverb), Chorus, Phaser, Drive, Auto Filter,
  Limiter, Stereo Width, Tremolo — chacun avec presets ; chaîne par piste + master.
- **Automation** : bouton « A » d'une piste → enveloppe de volume (re-cliquer : panoramique) ;
  clic = ajouter un point, glisser = déplacer, double-clic = supprimer.
- **Mixer** (page Mix) : volume, pan, mute/solo/arm, inserts, vu-mètres, master.
- **Audio** : import (WAV, MP3, FLAC, OGG, AIFF, M4A) par glisser-déposer depuis le Finder ou le
  navigateur de fichiers (avec pré-écoute), enregistrement micro sur piste audio armée, gain et
  fondus de clip.
- **MIDI** : claviers MIDI branchés (tous les ports, relancer la détection dans les réglages),
  « musical typing » au clavier d'ordinateur (bouton ⌨ ou Caps Lock), enregistrement MIDI sur
  piste instrument armée.
- **Fichiers** : projets = dossiers `NomDuMorceau.dawdaw/` (`project.json` + `Audio/` où les
  fichiers sont copiés), export WAV 16/24/32 bits (morceau ou boucle).
- **Annuler/rétablir** illimité (200 pas).

Raccourcis : bouton « ? » en haut à droite.

## Architecture

```
src/                         Frontend React (TypeScript)
  lib/types.ts               Le document projet (positions en temps = beats/noires)
  lib/store.ts               Store zustand : projet immuable (immer), historique, sélection, UI
  lib/actions.ts             Opérations d'édition (une fonction = un pas d'annulation)
  lib/engine.ts              Pont vers le moteur : synchro projet, transport, prises MIDI/audio
  lib/backend.ts             Appels Tauri (+ mock navigateur)
  lib/catalog.ts             Catalogue d'instruments/effets fourni par Rust
  lib/patterns.ts            Boucles MIDI intégrées (navigateur « Loops »)
  lib/templates.ts           Modèles de nouveau projet
  components/…               TopBar, Browser, Arrange, PianoRoll, Sound, Mixer, dialogues

crates/daw-core/src/         Moteur Rust partagé (bureau + navigateur)
crates/daw-wasm/src/lib.rs   Moteur en WebAssembly (API C minimale, sans wasm-bindgen)
public/engine-worklet.js     AudioWorklet qui héberge le moteur wasm
src/lib/webEngine.ts         Web Audio, Web MIDI, import et export dans le navigateur
src-tauri/src/plugins/       Hôte Audio Unit (FFI AudioToolbox, fenêtre d'édition Cocoa)
src-tauri/src/               App de bureau
  lib.rs                     Commandes Tauri, threads de statut/enregistrement, MIDI
  model.rs                   Sous-ensemble du document utile au moteur (serde)
  engine/controller.rs       Thread de contrôle : diff du projet → commandes (toutes les allocations ici)
  engine/mod.rs              Moteur temps réel (thread audio) : transport, séquenceur, mix
  engine/track.rs            Piste côté audio : notes, clips audio, effets, gains
  audio_io.rs                Périphériques cpal (sortie, entrée), enregistreur WAV
  assets.rs                  Décodage (symphonia), WAV (hound), forme d'onde
  project_io.rs              Sauvegarde/chargement, rendu hors-ligne (export)
  dsp/                       Instruments et effets + registre
  dsp/sampler.rs             Sampler et Sample Kit (samples embarqués)
  dsp/sample_data.rs         Table des samples (générée par tools/import_hichord.py)
samples/ (dans daw-core)     WAV + manifest.json + NOTICE.md
tools/                       import_hichord.py, measure_pitch.py
```

Principe : le frontend possède le document. À chaque modification, il envoie le projet
(au plus toutes les 16 ms) ; `Controller::sync` calcule ce qui a changé et pousse des commandes
dans une file sans verrou (`rtrb`) vers le thread audio. Les objets remplacés repartent par une
file « poubelle » pour être libérés hors du thread audio. Position, vu-mètres et charge CPU
remontent via des atomiques, émis vers l'UI à ~30 Hz.

### Ajouter un instrument ou un effet

1. Créez `src-tauri/src/dsp/effects/mon_effet.rs` avec un `pub static SPEC: DeviceSpec`
   (paramètres, presets) et une struct qui implémente `Effect` (ou `Instrument`).
2. Déclarez-le dans `dsp/effects/mod.rs` (`SPECS` et `create`) ou `dsp/registry.rs`.
3. C'est tout : l'UI génère les potards à partir de la spec (groupes, unités, choix, on/off).
   Lancez `cargo test --lib dump_catalog -- --ignored` pour rafraîchir le catalogue du mode
   navigateur (`src/lib/catalog.fixture.json`).

## Limites connues

- Plugins : Audio Units seulement (macOS). Vital, comme la plupart des plugins, existe aussi en
  AU. Les formats VST3 et CLAP pourront s'ajouter derrière la même interface
  (`daw_core::dsp::registry::ExternalDevices`). Pas de plugins dans la version navigateur.
- Automation limitée au volume et au pan des pistes ; pas de pitch-bend, pas d'étirement temporel de l'audio (un clip audio garde
  sa vitesse si le tempo change), pas de plugins VST/AU.
- Pas de monitoring d'entrée pendant l'enregistrement audio ; la latence d'enregistrement se
  compense à la main (réglages → « Recording offset »).
- Le limiteur introduit ~3 ms de latence non compensée.
