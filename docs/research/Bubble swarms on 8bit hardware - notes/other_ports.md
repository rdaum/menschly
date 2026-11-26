# Bubble Bobble home ports (1987-1990): how bubbles, enemies and players were rendered (ZX Spectrum, Amstrad CPC, MSX, Atari ST, Amiga, Sega Master System)

Scope note: C64 and NES were left out on purpose. Several fetches were blocked (403 or bot protection) on CPCWiki, StrategyWiki, Sega Retro, TCRF, Lemon Amiga and the Spectrum Computing review page. Where that happened, the claims below come from search-result excerpts of those pages, and the entry says so. **I found no disassembly or developer write-up that describes the sprite engine of any of these six ports.** Most of the technical detail is therefore reviewer observation plus inference.

## ZX Spectrum and Amstrad CPC (no hardware sprites): how were bubbles drawn, how many, at what frame rate?

### Takeaway
Software Creations made both ports for Firebird in 1987. On the Spectrum, sprites are monochrome and "see-through" (transparent), drawn over black backgrounds with coloured platforms. This avoids colour clash, at the cost of occasional slowdown in busy moments. The CPC port has chunky, colourful graphics and some sluggishness. I found nothing that documents the sprite routines, bubble limits or frame rates of either port.

### Cited Findings
- **Spectrum credits:** programming by Mike Follin, graphics by Andrew R. Threlfall, music by Tim Follin (Software Creations). Published by Firebird, 1987. — [Spectrum Computing entry](https://spectrumcomputing.co.uk/entry/722); [Arcade-History](https://www.arcade-history.com/game/78118/)
- **Parallel development:** David Broadhurst (Software Creations) says the rest of the team worked on the ZX Spectrum, C64 and Amstrad CPC versions while he did the 16-bit ports. It was "an arcade conversion with absolutely no documentation": the team played the arcade game repeatedly and photographed the screen with a 35mm camera. — [David Broadhurst project page](https://www.davidbroadhurst.me/projects/bubble-bobble)
- **Crash #45 (Oct 1987), reviewer Ricky:** "The cute little characters move very well, the platforms are very colourful and there's no clash visible anywhere." The Comments box says "plain backgrounds but supercute characters"; graphics scored 87%. — [CRASH 45 review](https://www.crashonline.org.uk/45/bubble.htm)
- **Crash Smash (issue 92, 1991):** "The small but detailed monochrome graphics make the action easy on the eyes". Taken from a search excerpt of the Spectrum Computing review page; the page fetch failed. — [Spectrum Computing reviews](https://spectrumcomputing.co.uk/zxsr.php?id=722)
- **Retrospective description:** "part-monochrome, displaying see-through characters on colourful platforms on black backgrounds, with just the odd bit of slowdown when things get frantic." — [RetroArcadia](https://retroarcadia.blog/2024/08/21/my-life-with-bubble-bobble-on-spectrum-st-arcade-everywhere/)
- **Contrary opinion:** Spectrum sprites are "quite small and ill-defined". — [Retro Sanctuary comparison](https://retro-sanctuary.com/Compaisons-Bubble-Bobble-3.html)
- **Mike Follin interview (1997):** he named the Spectrum's worst design point as "the absence of hardware assistance when it came to doing things like sprites or scrolling." He gives no details of the Bubble Bobble engine. — [World of Spectrum interview](https://worldofspectrum.net/interviews/FollinMike.htm)
- **CPC credits:** developed by Software Creations, published by Firebird in 1987. The release date and the Ocean 1988 re-release come from a StrategyWiki search excerpt. — [Games Database](https://www.gamesdatabase.org/game/amstrad-cpc/bubble-bobble); [StrategyWiki ports](https://strategywiki.org/wiki/Bubble_Bobble/Ports)
- **CPC programmer:** one source credits John Pickford, with Threlfall on graphics and Tim Follin on sound. I found no second source. — [The King of Grabs (CPC)](https://thekingofgrabs.com/2021/11/02/bubble-bobble-amstrad-cpc/)
- **CPC look and speed (The King of Grabs):** "one of the chunkiest and least defined" conversions, but colourful, and it "suffers a little from sluggishness in places". — [The King of Grabs (CPC)](https://thekingofgrabs.com/2021/11/02/bubble-bobble-amstrad-cpc/)
- **CPC look and speed (forum poster):** the graphics are "not high resolution [but] sensibly coloured", and the game "plays quite well and the controls are responsive". — [Spectrum Computing forum "Across the formats"](https://spectrumcomputing.co.uk/forums/viewtopic.php?t=6874)
- **CPC fan remake:** BB4CPC is a fan remake by César Nicolás González (cngsoft), not the 1987 port. Its reviewers note "a touch of sprite flicker". Flicker on the CPC is documented only for this remake. — [Classic Replay](https://classicreplay.blogspot.com/2015/12/bubble-bobble-4cpc.html); [EAB thread](https://eab.abime.net/showthread.php?t=62729)

### Inferences
- **Spectrum colour handling (inference):** the "monochrome", "see-through" sprites on a black background, with "no clash visible", point to one approach. Sprites are probably drawn by OR/XOR into the bitmap and inherit the ink colour of whatever cell they cross. The play area is black paper, so a single-colour sprite never needs its own attribute and clash stays invisible. Colour lives only in the static platform cells. No source confirms how the sprites are drawn or whether they are masked.
- **Spectrum positioning (inference):** reviewers say the characters "move very well", which suggests pixel-level movement rather than movement snapped to 8x8 character cells. This is not confirmed.
- **Spectrum frame rate (inference):** "odd bit of slowdown when things get frantic" means a variable frame rate in which the whole game slows down. The game does not drop or flicker objects instead. I found no fps figure.
- **CPC (inference):** "chunky" graphics fit the CPC's low-resolution 4-colour or 16-colour modes. In those modes each byte holds 2 or 4 pixels, so software sprites are costly to draw. This would explain the sluggishness. I found no source naming the screen mode.

### Gaps
- No disassembly or technical write-up for either 8-bit port. I could not confirm the maximum number of bubbles, whether the sprites are XOR or masked, whether the background is restored by redrawing or by saving it, or the frame rate.
- Mike Follin's YouTube interview (https://www.youtube.com/watch?v=rnlNDCaqZcQ) was not reviewed and may contain engine details.
- CPCWiki (https://cpcwiki.eu/zone/reviews/title/222/default.htm) returned 403.

## MSX: sprite flicker scheme and use of background tiles for bubbles

### Takeaway
The official MSX version is an **MSX2-only** 128KB MegaROM. Taito made it in Japan in 1987; it is not a UK conversion. That means the V9938 video chip rather than the TMS9918, so the question's "4 per line" premise may not apply. Reviewers report sprite flicker or "tearing" in busy scenes and severe slowdown in two-player mode. Community patches add a turbo mode for MSX2+ and turbo R machines.

### Cited Findings
- **Release:** MSX2, Taito, 1987, MegaROM (128kB), product code TMS-02, Japan. — [Generation MSX](https://www.generation-msx.nl/software/taito/bubble-bobble/release/998/)
- **Staff:** producer Shōji Takahashi; designer Tsutomu Takase; programmers Masahiro Atsuji, Santa Satou and T. Kuriyama; graphics Sachiko Yamana; sound Hisayoshi Ogura. Per a search excerpt of uvlist. — [uvlist](https://www.uvlist.net/game-16355-Bubble+Bobble)
- **Reviewer view:** the best 8-bit home-computer port, with sprite tearing when the screen gets busy, but "doesn't seem to suffer from any slowdown". — [The King of Grabs (MSX)](https://thekingofgrabs.com/2021/11/04/bubble-bobble-msx/)
- **Two-player slowdown:** a community patch page says the game "suffers from severe slowdown in two players mode". The patch (v0.96) fixes a wrong-facing dragon sprite, speeds the game up slightly and adds a turbo mode for turbo R and MSX2+. — [MSX Resource Center patch download](https://www.msx.org/downloads/bobble-bubble-improvement-patches-v096)
- **Flicker on MSX2 in general:** in a forum thread on MSX2 multicolour sprites, flicker is named as the main problem of that approach. (Search excerpt; not specific to this game.) — [MSX.org "Simply disappointed" thread](https://www.msx.org/forum/msx-talk/software-and-gaming/simply-disappointed)
- **Pace:** "plays at a relatively fast pace". — [Spectrum Computing forum](https://spectrumcomputing.co.uk/forums/viewtopic.php?t=6874)

### Inferences
- **Hardware (inference, uncited background knowledge):** the MSX2 V9938 has sprite mode 2, with 32 sprites and up to 8 per scanline, each line of a sprite having its own colour. Sprites can be OR-combined to get more colours. Colour-stacking two sprites per object halves the usable per-line budget. This would explain flicker in busy scenes even with 8 per line.
- **Flicker scheme:** "tearing" (pieces of sprites missing) suggests the game rotates sprite priority to share the per-line limit. This is inference only.
- **Bubbles as background tiles:** I found no source saying whether bubbles are drawn as background tiles on MSX.

### Gaps
- No technical analysis of the MSX2 sprite allocation, the bubble cap or the flicker rotation.
- Whether an MSX1 version exists: I found none.

## Atari ST (no sprites) and Amiga (hardware sprites + blitter): software/blitter bobs, frame rate, bubble counts

### Takeaway
Both 16-bit versions were coded by David J. Broadhurst alone at Software Creations, with graphics by Andrew Threlfall. He wrote them in 68000 assembly and **developed on the ST first**, sending compiled code to the Amiga over a serial link. The Amiga version is effectively an ST port. A remake's comparison implies it ran at about 25 fps in 16 colours. Neither the developer nor any reviewer I found describes whether the Amiga used hardware sprites or blitter bobs.

### Cited Findings
- **Credits:** programming by David J. Broadhurst; developer Software Creations; publisher Firebird; ST release 1987. — [Atarimania ST entry](https://www.atarimania.com/games/atari-st-games-bubble-bobble-8820)
- **Development process:** Broadhurst learned 68000 assembly at college and was "on my own with the 16-bit ports". He built the game on the Atari ST and sent compiled code to the Amiga over a serial cable. The ST version was finished before he began the Amiga port. For the US release the main change was switching the Amiga refresh rate to 60 Hz for NTSC, which he could not test because he had no NTSC machine. — [David Broadhurst project page](https://www.davidbroadhurst.me/projects/bubble-bobble)
- **Amiga credits:** coder Broadhurst, graphics Andrew R. Threlfall, music David Whittaker (search excerpt). Lemon Amiga's review calls it an ST port with "minor framerate issues" and iffy collision detection (search excerpt; the fetch returned 403). — [Lemon Amiga](https://www.lemonamiga.com/games/details.php?id=199)
- **Frame rate and colours by comparison:** the fan remake Tiny Bobble promises "50 fps instead of 25, 32 colours instead of 16" compared with the original Amiga version. The 25 fps and 16 colours are implied by that comparison, not stated outright. — [Indie Retro News on Tiny Bobble](https://www.indieretronews.com/2020/08/tiny-bobble-arcade-quality-port-of.html)
- **Critical opinion:** the ST and Amiga versions are "simply more colorful versions of 8-bit Bubble Bobble" rather than pushing 16-bit hardware (search excerpt). — [Lilura blog](https://lilura1.blogspot.com/2022/04/Bubble-Bobble-Amiga-1988-Software-Creations-David-J-Broadhurst.html)
- **ST reception:** Atari ST User 8/10 (March 1988); ST Action 76% (November 1988), per search excerpts. — [Atarimania](https://www.atarimania.com/game-atari-st-bubble-bobble_8820.html)
- **Amiga date conflict:** sources disagree on the Amiga release date (1987, 1988 or 1989). — [Lilura blog](https://lilura1.blogspot.com/2022/04/Bubble-Bobble-Amiga-1988-Software-Creations-David-J-Broadhurst.html); [The King of Grabs (Amiga)](https://thekingofgrabs.com/2021/11/04/bubble-bobble-amiga/)

### Inferences
- **ST rendering (inference):** the ST has no sprites or blitter (in 1987 machines), so all objects must be CPU-drawn masked software sprites, with the background restored each frame. A 25 fps cap (every other 50 Hz frame) is a typical way to fit many software sprites into the frame budget.
- **Amiga rendering (inference):** the game was built on the ST and moved over with few changes; the Amiga also keeps 16 colours and 25 fps. This strongly suggests the Amiga version reused the ST's CPU software-sprite renderer rather than a design built around the Amiga's hardware sprites or blitter. Not confirmed.
- **Variable frame rate (inference):** "minor framerate issues" suggests the rate drops below 25 fps under heavy bubble load, rather than objects being dropped.

### Gaps
- No source states the maximum number of bubbles or enemies, or the exact drawing method (CPU vs blitter, hardware sprites).
- I found no EAB or Atari-forum technical thread, and no ST Format text.

## Sega Master System / "Final Bubble Bobble": flicker and tile-based bubbles

### Takeaway
Taito developed it and Sega published it: Japan, 2 July 1988, as *Final Bubble Bobble*; Europe 1991; Brazil as *Dragon Maze*. It has 200 levels. Moving objects (dragons, enemies, bubbles, fruit) are hardware sprites under the SMS limit of 8 sprites per scanline. The game spreads flicker by **rotating the sprite table, reportedly in steps of 7 sprites**. Static event items such as crowns are drawn into the background tilemap. Reviewers disagree on how bad the flicker and slowdown are.

### Cited Findings
- **Release:** developer Taito, publisher Sega; Japanese release in 1988 as Final Bubble Bobble. — [Sega-16 review](https://www.sega-16.com/2026/11/final-bubble-bobble/); [Wikipedia list of Master System games](https://en.wikipedia.org/wiki/List_of_Master_System_games)
- **What flickers:** forum user TinpotGamer observed that "everything seems subject to standard sprite disappearance on scan lines where the maximum of 8 is exceeded". The exception is auto-spawning event items, which appear to be background tiles. — [SMS Power! forum thread](https://www.smspower.org/forums/15251-BubbleBobbleBadGraphicalDevelopmentDecisions)
- **Flicker rotation:** Calindro, the author of the Emulicious emulator, viewed the sprite table and reported: "It seems to cycle them at steps of 7 sprites". Calindro also confirmed that event items such as crowns are in the tilemap. — [SMS Power! forum thread](https://www.smspower.org/forums/15251-BubbleBobbleBadGraphicalDevelopmentDecisions)
- **Why not tiles for moving objects:** Kagesan said moving objects should generally not be background tiles: "writing to the name table a lot is relatively slow and you could run into timing problems quickly". Forcing priority would make the remaining sprites "even more prone to flickering". sverx suggested that anything not moving continuously could become background, "better if grid-aligned". — [SMS Power! forum thread](https://www.smspower.org/forums/15251-BubbleBobbleBadGraphicalDevelopmentDecisions)
- **Reviewer split, minimal flicker:** Sega-16 says it "floods the screen with dozens of moving sprites with minimal flicker and slowdown", over "a black void". The big arcade heart blocks become "2×2 stacks of the smaller heart blocks", and the platforms lose their drop shadow. — [Sega-16 review](https://www.sega-16.com/2026/11/final-bubble-bobble/)
- **Reviewer split, major flicker:** "Flicker is also a pretty major problem due to sprite limits... not particularly surprising given the massive amount of bubbles on screen the second you get that rapidfire candy". The reviewer adds that it "doesn't look nearly as bad on an actual Master System running at a native 60FPS" as in the page's 20 fps GIFs. — [Overclocked Sega Ages](https://overclockworked.neocities.org/segaages/bubble/)
- **Mean Machines Sega:** scored it 85% and noted slowdown and sprite flicker (search excerpt). — [SMS Power! reviews](https://www.smspower.org/Reviews/BubbleBobble-SMS-MeanMachinesSegaGamesIndex)
- **Bonus numbers:** these do not flicker, unlike in other conversions; the reviewer also notes some sprite tearing. — [The King of Grabs (SMS)](https://thekingofgrabs.com/2021/11/05/bubble-bobble-sega-master-system/)

### Inferences
- **Rotation step (inference):** a rotation step of 7, a prime that does not divide 64, would move every sprite through every priority position over successive frames. This would spread the dropout evenly across objects, consistent with Kagesan's suspicion that the developers spread flicker deliberately.
- **Hybrid split (inference):** the tile/sprite split looks deliberate: static or rarely-moving pickups go in the background, and everything that moves is a sprite.
- **Tile-based bubbles:** I found no evidence that bubbles are ever drawn as background tiles. The thread suggests they are not.

### Gaps
- No disassembly that gives the bubble cap or the sprite size per object (e.g. 2x2 sprites of 8x8 or 8x16).
- No measured slowdown data.
- Sega Retro and TCRF fetches failed.
