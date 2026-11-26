# C64 Bubble Bobble (Firebird / Software Creations, 1987): how bubbles, enemies and players were rendered

Scope note: very little first-hand technical documentation of this port's renderer is reachable on the open web. The best primary source is Victor Widell's ("geon") reverse-engineering blog, which works from the actual 6510 code. A second disassembly write-up (emix8.org/bbdisasm) is cited by search engines, but its live site sat behind a hosting "security check" interstitial and the Wayback copy could not be fetched from this environment, so its contents are **unverified** here. Lemon64, MobyGames and Spriters Resource returned HTTP 403. The one known developer interview, with Ste Ruddy in Retro Gamer #37, is only available second-hand. Everything below is labelled by evidence strength.

Credits (context): coding by Stephen Ruddy, graphics by Andrew R. Threlfall, music by Peter Clarke — [Dave's Retro Forge remaster page](https://daves-retro-forge.itch.io/bubble-bobble-c64-remastered); developer Stephen Ruddy, Software Creations / Firebird — [C64-Wiki](https://c64-wiki.com/wiki/Bubble_Bobble).

## Q1. Were bubbles drawn as characters (soft sprites), as hardware sprites, or a mix? Did they move in character steps or smoothly?

### Takeaway
The engine certainly uses **both** hardware sprites and **software sprites drawn into character/colour RAM**: a reverse engineer working from the code says so directly. No source I could verify says which objects (bubbles, projectiles, enemies) are the software sprites, or whether they move in 8-pixel steps. The idea that bubbles specifically are character-based is plausible but unconfirmed.

### Cited Findings
- **Documented (code-level, reverse engineer):** In his analysis of the level-init routine, geon writes: "The software sprites that use the color ram all update it when drawn anyway." He also notes that during level transitions "The sprites are hidden and the color ram is set to black" (via `jsr $e740 ; Set all color ram to A.`), with hardware sprites toggled through `$d015`. The article does not name which game objects are software sprites. — [geon: Fixing The C64 Bubble Bobble Continue Bug](https://geon.github.io/programming/2025/10/07/bubble-bobble-c64-continue-bug-fix)
- **Documented (code-level):** Large objects are built from several hardware sprites. The big watermelon bonus "uses 2x2 hardware sprites," and "Other multi-sprites like this are laid out sequentially in memory, left-to-right, top-to-bottom." — [geon: I wrote the worlds worst emulator](https://geon.github.io/programming/2025/07/24/bubble-bobble-c64-rng-emulator)
- **Documented (code-level):** The level graphics are character-based. Levels are 32×25 tiles, the platform layout is a 1-bit-per-tile bitmap, and the decorative side graphics are 2×2 tiles of 8 bytes each, "written into the charset when a level loads." Bubbles follow per-tile "wind currents" held in a one-byte-per-tile buffer at `$8500`, in four directions (up/right/down/left). — [geon: Parsing the c64 Bubble Bobble Wind Currents](https://geon.github.io/programming/2025/01/05/bubble-bobble-c64-wind)
- **Forum statement (unverified, second-hand via search snippet):** A Lemon64 poster names Bubble Bobble as a game that bases "the whole game on a sprites + softwaresprites combination." I could not read the thread itself (HTTP 403). — [Lemon64 thread t=55524 "Nes-like blinking to show more than 16 sprites?", p.2](https://www.lemon64.com/forum/viewtopic.php?amp=&amp=&start=15&t=55524)
- **Contemporary review (no technique detail):** Zzap!64 #30 (Oct 1987) praises "Extremely 'cute' and highly detailed sprites" and notes "Bubbles always mass at the top of the screen." — [Zzap!64 #30 review via Everygamegoing](https://everygamegoing.com/larticle/Bubble-Bobble-000/28482)
- **Player observation (low reliability):** A user review says that on the C64 "the bubble flashes on and off around the character." — [Listal review](https://www.listal.com/viewentry/609880)

### Inferences
- *Inference:* Software sprites that write colour RAM are, by definition, character-cell objects. A character-cell object either snaps to the 8×8 grid or uses pre-shifted character graphics. Because bubbles in the top-of-screen "mass" ride a per-tile wind grid, they are the most natural candidates for the software-sprite layer. That would leave hardware sprites free for the players, enemies and big bonus items. This is a reasonable hypothesis, not a documented fact.
- *Inference:* The per-tile wind map (one direction byte per 8×8 cell) suggests bubble drift is resolved on the character grid. That alone does not prove the bubbles are *drawn* at 8-pixel steps.

### Gaps
- No verified source names which objects are software sprites, or whether they move in 1-pixel or 8-pixel steps.
- The emix8.org/bbdisasm write-up (search-snippet title: a "raster-synced sprite routine"; it references "the original commented source code"; the asset list includes `bubbleblend.png`, `buffers.png`, `rasterscan.png` and `frame.mp4`, per [Wayback CDX](http://web.archive.org/cdx/search/cdx?url=emix8.org/bbdisasm*)) would likely answer this. It could not be read here, and I could not even confirm whether it covers the C64 or the arcade Z80 code. **Recommended follow-up:** open https://web.archive.org/web/20250613002940/https://www.emix8.org/bbdisasm/ in a browser.
- Martin Piper's video "C64 Games memories – Bubble Bobble" (geon links to its code-analysis section at 14:15: https://youtu.be/O3eAtyBHOOc?t=855) probably shows the engine. Its content could not be extracted.

## Q2. Did the game use sprite multiplexing? How many hardware sprites did players, enemies and bubbles use? Was there flicker?

### Takeaway
I found no verified documentation of a raster-interrupt multiplexer in C64 Bubble Bobble. What is documented: hardware sprites exist, multi-sprite objects exist (the 2×2 watermelon), and a software-sprite layer supplements them. Some flicker exists in at least one situation: a fan hack's changelog lists a fix for "sprite flickering in the Secret Room."

### Cited Findings
- Hardware sprites are enabled and disabled through `$d015` around level transitions. — [geon continue-bug post](https://geon.github.io/programming/2025/10/07/bubble-bobble-c64-continue-bug-fix)
- The bonus watermelon takes four hardware sprites (2×2), which on its own is half of the VIC-II's eight. — [geon RNG post](https://geon.github.io/programming/2025/07/24/bubble-bobble-c64-rng-emulator)
- The Bubble Bobble C64 Remastered v1.1a changelog (a hack of the original PAL program by Davide Bottino and Victor Widell) lists fixes for "sprite flickering in the Secret Room" and "corrupted graphics for the projectiles of Invader and Drunk." — [Dave's Retro Forge remaster page](https://daves-retro-forge.itch.io/bubble-bobble-c64-remastered)
- The remaster developer says high-resolution sprites "would require a major code rewrite." — [same page, comments](https://daves-retro-forge.itch.io/bubble-bobble-c64-remastered)
- General background (not specific to this game): the VIC-II cannot multiplex sprites horizontally, only by reusing them further down the screen, and late raster IRQs in a multiplexer cause flicker and slowdown. — [Codebase64: sprite multiplexing](https://codebase.c64.org/doku.php?id=base%3Asprite_multiplexing); [Cadaver: Sprite multiplexing](https://cadaver.github.io/rants/sprite.html)
- A search snippet attributes the title "raster-synced sprite routine" to the emix8 Bubble Bobble disassembly. — unverified, see Q1 Gaps.

### Inferences
- *Inference:* Two players, up to around seven enemies per level, enemy projectiles and big bonus items can easily exceed eight hardware sprites. A hybrid design (hardware sprites for some objects, character "software sprites" for others) fits the documented evidence and is the usual 1987 way to dodge multiplexer limits on a single non-scrolling screen. Whether a vertical multiplexer was *also* used remains unconfirmed.
- *Inference:* The "bubble flashes on and off around the character" player report could reflect software-sprite redraw (character cells being restored and redrawn) or sprite-priority effects. It is not evidence of multiplexer flicker.

### Gaps
- No exact count of hardware sprites per object type (players, enemies, bubbles) was found.
- No verified statement on whether a raster multiplexer exists.

## Q3. How many bubbles could be on screen at once, and what happened with many (slowdown, flicker, disappearing)?

### Takeaway
I found no documented bubble cap or overload behaviour for the C64 version. The only related evidence is anecdotal: fans find the bubble spawn rate and bubble speed lower than the arcade's.

### Cited Findings
- Remaster developer: "Bubble spawn rate and bubble speed are definitely something I would improve, but at the moment there's no easy fix." A commenter asks for "more water bobble spawn" on level 13. — [Dave's Retro Forge remaster page](https://daves-retro-forge.itch.io/bubble-bobble-c64-remastered)
- The developer judges the C64 version "a solid recreation of the arcade original" but says "the simplified mechanics mean that the spirit of the original didn't fully carry over." — [same](https://daves-retro-forge.itch.io/bubble-bobble-c64-remastered)
- Zzap!64: "Bubbles always mass at the top of the screen"; "heaps of things whizzing all over the screen." The review mentions no slowdown or flicker. — [Zzap!64 #30 via Everygamegoing](https://everygamegoing.com/larticle/Bubble-Bobble-000/28482)

### Inferences
- *Inference:* A spawn rate that is hard to change ("no easy fix") hints that object-slot limits or the timing budget are baked into the engine, for example a fixed-size bubble table. This is speculative.

### Gaps
- No maximum bubble count, and no description of what happens at the limit (oldest bubble popped, new bubble refused, slowdown). Answering this needs the disassembly or the Martin Piper video.

## Q4. Update rates: every frame, or reduced/staggered? Any frame-rate figures?

### Takeaway
The game logic runs on the PAL 50 Hz frame tick: the RNG and game-over checks are documented as running "every frame." No source documents staggered object updates or the effective animation or movement rates of bubbles.

### Cited Findings
- "At the player select screen, the RNG is called every frame"; geon reasons from "50 (pal) frames per second." — [geon RNG post](https://geon.github.io/programming/2025/07/24/bubble-bobble-c64-rng-emulator)
- The game-over check runs "every frame." — [geon continue-bug post](https://geon.github.io/programming/2025/10/07/bubble-bobble-c64-continue-bug-fix)
- The game is PAL-timed. NTSC machines or emulation "may cause issues such as inconsistent or incorrect game speed," and an NTSC player reports that "the tempo slows down to normal between levels, and speeds up in game." — [Dave's Retro Forge remaster page](https://daves-retro-forge.itch.io/bubble-bobble-c64-remastered)
- Fans feel "Player characters could use a slight speed boost." — [same](https://daves-retro-forge.itch.io/bubble-bobble-c64-remastered)

### Inferences
- *Inference:* The NTSC behaviour (normal tempo between levels, faster during play) suggests in-game logic is tied to the raster or frame interrupt rather than a wall-clock timer, so the game runs at 60 Hz on NTSC. It also suggests the in-game loop normally fits within one frame on PAL. Not confirmed.

### Gaps
- No figures for movement speeds, animation rates, or whether bubbles, enemies and software sprites update on alternate frames.

## Q5. Developer interviews, magazine technical articles, disassemblies and videos

### Takeaway
The only known developer interview is with Ste Ruddy in Retro Gamer #37. Second-hand summaries cover his *process* (no arcade source code), not the renderer. Code-level material comes from geon's blog posts, an unverified emix8 disassembly page, and a Martin Piper YouTube video.

### Cited Findings
- Per MobyGames trivia, citing the Retro Gamer 37 interview: Ruddy had no access to the arcade source code, worked out the details by playing the original, and had only untranslated Japanese design documents. He later attempted an unfinished Java-phone port. — MobyGames trivia page (HTTP 403 when fetched; content taken from search snippets): https://www.mobygames.com/game/3ds/bubble-bobble/trivia
- geon (Victor Widell) has published three code-level posts: wind currents with annotated 6510 disassembly ([link](https://geon.github.io/programming/2025/01/05/bubble-bobble-c64-wind)), the RNG and powerups ([link](https://geon.github.io/programming/2025/07/24/bubble-bobble-c64-rng-emulator)), and a continue-bug fix ([link](https://geon.github.io/programming/2025/10/07/bubble-bobble-c64-continue-bug-fix)). He also maintains a level-parsing tool at https://github.com/geon/bb-construction-set and posted an earlier level-data rip on [Reddit r/c64](https://www.reddit.com/r/c64/comments/196mmau/i_ripped_the_level_data_of_bubble_bobble/).
- geon links a Martin Piper YouTube code analysis: https://youtu.be/O3eAtyBHOOc?t=855 (page title "C64 Games memories – Bubble Bobble"; content not extractable here). — [geon wind post](https://geon.github.io/programming/2025/01/05/bubble-bobble-c64-wind)
- The emix8.org/bbdisasm disassembly write-up is indexed. Archived snapshots exist for 2024-02-07 through 2025-06-13, with images named bubbleblend, buffers, rasterscan and spectrumanalyser and a frame.mp4 video. — [Wayback CDX listing](http://web.archive.org/cdx/search/cdx?url=emix8.org/bbdisasm*). The content and target platform are unverified.
- Zzap!64 #30 (1 Oct 1987) gave 97% and a Gold Medal, with Graphics at 84%. It covers aesthetics only. — [Everygamegoing](https://everygamegoing.com/larticle/Bubble-Bobble-000/28482); [Zzap!64 #30 scan text, archive.org](https://archive.org/stream/zzapp_64_issue_030_600dpi/Zzap640030Hi_djvu.txt)

### Inferences
- *Inference:* geon has worked on the code most deeply (he is also the remaster's "hacking & tools" person) and is the most likely person able to answer the bubbles-as-software-sprites question definitively.

### Gaps
- I could not read the Retro Gamer #37 interview text, so I don't know whether Ruddy discusses rendering.
- I found no Codebase64 or CSDb write-up specific to this game's engine.
- Lemon64 threads (t=55524, t=80233) were inaccessible (HTTP 403).
