# NES/Famicom Bubble Bobble: rendering many bubbles and enemies within NES sprite limits

Scope note: public technical documentation for this specific port is thin. I found no NES disassembly, no nesdev forum thread, no TASVideos game-resources page and no developer interview about its object engine. Most of what follows is either (a) general NES hardware facts, (b) player and reviewer reports of flicker and slowdown, or (c) a small amount of RAM evidence from a hobbyist Lua bot. Anything about the engine's internals beyond that is labelled as inference.

Source-quality warnings:
- The TCRF page for the NES game (https://tcrf.net/Bubble_Bobble_(NES)) returned prompt-injection text instead of article content when fetched. Nothing from that fetch is used here. One search-engine snippet attributed to it (that the FDS version is "identical to the one seen in the original arcade version", context unclear) is not verified.
- emix8.org/bbdisasm (a Bubble Bobble disassembly, probably of the arcade original) and the RetroPie flicker thread were both behind bot-check pages and could not be read.
- StrategyWiki's NES page returned HTTP 403. Its "Changes from the arcade" content below comes only from a search snippet.

## Hardware budget and how the game uses sprites (metasprite size, 8x8 vs 8x16 mode)

### Takeaway
The NES gives 64 sprites, 8 per scanline, in either 8x8 or 8x16 mode. I found no source saying which mode Bubble Bobble uses or how its metasprites are built. A hobbyist bot script reads the player and enemies straight out of the OAM shadow buffer at $0200. Its fixed slot offsets hint at 16x16 characters made from 8x8 hardware sprites, but that is inference.

### Cited Findings
- The NES PPU can use 8x8 or 8x16 sprites, with 64 sprites in either mode. 8x16 sprites can take tiles from both pattern tables. — [NESdev Wiki: Sprite size](https://nesdev.org/wiki/Sprite_size)
- The NES draws at most 8 sprites per scanline. Flicker comes from games rotating the sprite order in software so the dropped sprites change each frame. It had to be coded by hand in each game. — [Matt Hughson thread (NES developer)](https://threadreaderapp.com/thread/1605346030505308160); [SVG.com: Why your old NES games flickered](https://svg.com/1147998/this-is-why-your-old-nes-games-flickered)
- Cartridge: Mapper 1 (Nintendo MMC1). Known dumps are the U and E versions. A Famicom Disk System version also exists. — [Data Crystal: Bubble Bobble](https://datacrystal.tcrf.net/wiki/Bubble_Bobble)
- The FCEUX watch file in the DPS2004/bubblebot repo labels these addresses: player Y = $0200, player X = $0203, enemy 1 Y = $0210, enemy 2 Y = $0228, enemy 3 Y = $0240. The bot script reads the player position from $0203/$0200 (adding offsets of −1 and +15) and the enemy Y from $0240 (+15). — [DPS2004/bubblebot (GitHub), files `Bubble Bobble (USA).wch` and `bubblebot.lua`](https://github.com/dps2004/bubblebot)
- The Data Crystal RAM map lists only gameplay counters and state. Examples: bubbles blown $0435, empty bubbles popped $0436, fire/water/lightning bubbles popped $0438–$0440, appearance of elemental bubbles $04CE bits 5–7, enemy anger level $0492, frame counter $001F. It has no object-slot table, bubble count or OAM documentation. — [Data Crystal: Bubble Bobble (NES, FDS) RAM map](https://datacrystal.tcrf.net/wiki/Bubble_Bobble_(NES,_Famicom_Disk_System)/RAM_map)

### Inferences
- $0200–$02FF is the usual OAM shadow page that NES games DMA to the PPU through $4014. Bytes 0 and 3 of an OAM entry are Y and X, which matches the labels "player y = $0200" and "player x = $0203". So the bot was reading hardware sprite entries directly, not a separate object table. (Inference from standard NES OAM layout plus the bubblebot labels.)
- If $0200 is the player's first sprite and an enemy starts at $0210, the player's metasprite takes 16 bytes, which is 4 OAM entries. That fits a 16x16 character built from four 8x8 sprites. With 8x16 sprites a 16x16 character would need only 2 entries. Enemy starts at $0210, $0228 and $0240 are 24 bytes (6 entries) apart, which doesn't divide cleanly into 16x16 characters. The labels may be approximate, or each block may include extra sprites. Weak evidence, unverified.
- If a bot can rely on a fixed OAM address for the player, the player's sprites may not be rotated through OAM, or may sit at a fixed position at least some of the time. That would mean partial rather than full OAM cycling. Unverified. The script author may simply have tolerated wrong readings on some frames.
- Using the +15 Y offset as a "feet" position suggests characters about 16 pixels tall, consistent with 16x16 objects. (Inference.)

### Gaps
- No source states whether PPUCTRL selects 8x8 or 8x16 sprites in this game. Checking the PPUCTRL bit 5 write in an emulator debugger, or the OAM viewer in Mesen/FCEUX, would settle it. Not done here, because I did not obtain the ROM.
- I found no documented OAM layout or sprite counts for the bubble, enemy or player metasprites.

## Were floating bubbles sprites or background tiles?

### Takeaway
I found no source that explicitly states how the NES port draws its bubbles. Players and reviewers consistently say bubbles flicker together with enemies and dragons while the background stays steady. On the NES that indicates bubbles are hardware sprites, but it is inference. This differs from the C64 port, where bubbles were reportedly part of the background graphics.

### Cited Findings
- Cubed3 review: "there is one glaring issue and that's the sprites often flicker. This can be attributed to the prevalence of bubbles and enemies, which is sometimes too much for the NES to handle." — [Cubed3 NES review](https://www.cubed3.com/games/reviews/nes/bubble-bobble)
- A user report on a RetroPie forum thread (search snippet only; the page itself was bot-gated): blowing too many bubbles makes the dragons and bubbles flicker, while the background is fine. — [RetroPie forum](https://retropie.org.uk/forum/post/36852)
- SEGA Versus comparison (search snippet): on NES, when action gets hectic the sprites flicker, and enemies and bubbles are the main culprits. — [SEGA Nerds: SEGA Versus Bubble Bobble](https://www.seganerds.com/2013/08/09/sega-versus-bubble-bobble/)
- Contrast with other 8-bit ports: in the C64 version, the bubbles are reportedly not animated anywhere near full frame rate and are part of the background graphics (search snippet from a forum discussion). — [Sleeping Elephant / Denial forum, "WIP: Bubble Bobble"](https://www.sleepingelephant.com/~sleeping/ipw-web/bulletin/bb/viewtopic.php?p=121015)
- The Spriters Resource has separate NES rips for "Bubble Types" and "Level Tiles". I did not open them, so they don't show which way bubbles are drawn. — [The Spriters Resource: Bubble Bobble (NES)](https://www.spriters-resource.com/nes/bublbobl/)

### Inferences
- On the NES, only sprites drop out on crowded scanlines. Background tiles never flicker. Several independent reports say bubbles flicker, so floating bubbles in the NES port are very likely OAM sprites, not nametable tiles. (Strong inference; no primary technical confirmation.)
- If bubbles were tiles they would move in 8-pixel steps or need attribute and nametable updates every frame. No source describes stepwise bubble motion on NES, which also fits sprite rendering. (Inference.)

### Gaps
- No primary documentation (disassembly, OAM capture or developer statement) confirms that bubbles are sprites. A PPU/OAM viewer session in an emulator would confirm it.
- Unknown whether any special bubbles (for example the water flood or large letter bubbles) use background tiles.

## Flicker / OAM rotation scheme, flicker visibility, and slowdown

### Takeaway
The game visibly flickers bubbles, enemies and dragons when many share scanlines, and it slows down a little when many bubbles are on screen. These are qualitative reports from reviewers and players. I found no description of the actual rotation algorithm and no measured frame rates.

### Cited Findings
- Flicker is the port's "one glaring issue". Bubbles and enemies are "sometimes too much for the NES to handle". — [Cubed3](https://www.cubed3.com/games/reviews/nes/bubble-bobble)
- SEGA Versus (search snippet): the NES frame rate is described as solid, but sprites flicker when things get hectic, much as on the Master System. — [SEGA Nerds](https://www.seganerds.com/2013/08/09/sega-versus-bubble-bobble/)
- GameFAQs user reviews (search snippets): "some slowdown and flickering in graphics when the screen becomes very busy" and "a little slowdown when there is a lot of bubbles on the screen". — [GameFAQs reviews, Bubble Bobble (NES)](https://gamefaqs.gamespot.com/nes/563392-bubble-bobble/reviews/45075)
- General mechanism: NES games rotate OAM priority each frame so the sprites beyond the per-line limit change from frame to frame, which turns permanent dropout into flicker. — [Matt Hughson thread](https://threadreaderapp.com/thread/1605346030505308160)
- An unverified snippet about the emix8 Bubble Bobble disassembly mentions "some kind of raster-aware sprite routine". The page was bot-gated, and it is most likely the arcade or another platform, not the NES. — [emix8.org/bbdisasm](https://emix8.org/bbdisasm)
- The NESdev Wiki list of sprite overflow and masking games does not mention Bubble Bobble. — [NESdev Wiki: Sprite overflow games](https://www.nesdev.org/wiki/Sprite_overflow_games)

### Inferences
- The flicker is described as shared among bubbles, enemies and dragons rather than as permanent dropout, so the game almost certainly uses some form of software OAM cycling. Whether it uses a per-frame rotating start index, alternating forward/reverse order, or something else is unknown. (Inference.)
- The slowdown reports point to CPU load from object logic, collision and metasprite building, not to the PPU, since sprite overflow on its own doesn't slow a game. That is the usual cause on NES, but it is not documented for this title. (Inference.)
- Hint from bubblebot: if the player's OAM slot is fixed (see the first section), dragons may get priority and appear in fixed low OAM slots while bubbles and enemies rotate. That would contradict the reports that dragons flicker too, so the evidence conflicts and the hint is weak. (Inference; unresolved.)

### Gaps
- I found no measured frame rate or lag-frame counts. TASVideos submission notes for the published runs ([TASVideos game 61](https://tasvideos.org/61G); [publication history](https://tasvideos.org/Games/PublicationHistory/61)) might mention lag, but I did not read them.
- I found no frame-by-frame analysis of the rotation pattern.

## How many bubbles can exist at once?

### Takeaway
No source documents a maximum bubble count for the NES version.

### Cited Findings
- The Data Crystal RAM map tracks cumulative bubble statistics ("Bubbles blown" $0435 and the various popped counters) but not live bubble slots or a cap. — [Data Crystal RAM map](https://datacrystal.tcrf.net/wiki/Bubble_Bobble_(NES,_Famicom_Disk_System)/RAM_map)
- Gameplay difference from the arcade (search snippet; the page returned 403): NES bubbles cannot be popped until they inflate to full size, whereas newly formed arcade bubbles could be popped at once. — [StrategyWiki: Bubble Bobble NES](https://www.strategywiki.org/wiki/Bubble_Bobble/NES_Walkthrough)

### Inferences
- With 64 OAM entries, 16x16 objects made of four 8x8 sprites would allow at most 16 visible objects per frame, or 32 if 8x16 sprites were used. That has to cover 2 players, up to roughly 6–7 enemies, items, bubbles and projectiles. So the game would need either a fairly small bubble cap or more objects than OAM can hold, relying on flicker across frames. This is arithmetic, not a documented figure.
- The NES rule that bubbles can't be popped until fully inflated may be a gameplay change related to object handling. That is speculative; no source links it to technical limits.

### Gaps
- Maximum simultaneous bubbles, enemy cap and object-slot count are all undocumented in what I found. They could be found by locating the object table in RAM with an emulator (for example by spamming bubbles and watching RAM).

## Disassemblies, ROM-hacking notes, TASVideos, nesdev and developer information

### Takeaway
I found no public NES disassembly. The available material is a Data Crystal RAM map of game statistics, a hobbyist Lua bot that reads OAM directly, and TASVideos run pages. No nesdev thread or developer interview about the engine turned up.

### Cited Findings
- Data Crystal has a game page (Mapper 1/MMC1, U and E dumps) and a RAM map limited to statistics, player lives and invincibility timers ($002E/$003F for P1, $0042/$0053 for P2), level ($0401), hurry-up timers ($040D/$040E), boss HP ($006F), score and password. — [Data Crystal game page](https://datacrystal.tcrf.net/wiki/Bubble_Bobble); [RAM map](https://datacrystal.tcrf.net/wiki/Bubble_Bobble_(NES,_Famicom_Disk_System)/RAM_map)
- DPS2004/bubblebot contains FCEUX Lua bot scripts with OAM-based player and enemy addresses, a level-geometry reader that scans the nametable at PPU $2060–$237F (tile $26 = non-solid), and RAM $0500–$07FF as a collision map in which byte $00 = solid. — [GitHub: DPS2004/bubblebot](https://github.com/dps2004/bubblebot)
- TASVideos has published runs. The current 2-player run is 24:37.70 (published 2010-02-04); earlier 1P and 2P runs are obsolete. A forum post notes that NES collision differs a lot from the arcade. I found no game-resources page. — [TASVideos game 61](https://tasvideos.org/61G); [Publication history](https://tasvideos.org/Games/PublicationHistory/61); [TASVideos forum](https://tasvideos.org/Forum/Topics/671)
- The Famicom (FDS) version came out in 1987 and the NES version in 1988. The English NES release uses different background tiles from the Japanese FDS conversion. — [The King of Grabs: Bubble Bobble NES](https://thekingofgrabs.com/2021/11/03/bubble-bobble-nes/)
- Large-scale lists of NES disassemblies (realdmx/retrore, qalle2/nes-disasm-misc, 6502disassembly.com) do not appear to include Bubble Bobble NES (based on search results; not checked exhaustively). — [6502disassembly.com](https://6502disassembly.com/); [realdmx/retrore](https://github.com/realdmx/retrore)

### Inferences
- The bubblebot collision map at $0500–$07FF suggests the game keeps a RAM copy of level solidity separate from the nametable, a common NES approach for fast tile collision. (Inference from the script's use.)

### Gaps
- No Displaced Gamers, Retro Game Mechanics Explained or similar technical video covering this port was found.
- No developer credits or interviews describing the NES/FDS engine were found.
- I did not check whether the FDS and cartridge versions differ in sprite handling.
- Recommended primary-research step: run the U ROM in Mesen, use the sprite/OAM viewer while spamming bubbles, and log the PPUCTRL sprite-size bit and the OAM DMA source ordering per frame. That would answer the mode, metasprite and rotation questions directly.
