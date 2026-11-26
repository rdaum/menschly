# Techniques for many moving objects on sprite-poor 8-bit hardware, and what the MEGA65 RRB offers

## Character/tile-based soft sprites, and objects that become background when they stop

### Takeaway
On machines with no sprites or too few, objects were drawn into the screen or character set. Snapping them to character cells is cheap but moves in coarse steps. Smooth motion needs pre-shifted copies (8 per graphic on a 1bpp Spectrum), which costs memory. Sources show C64 Gauntlet was character-based. I found no primary source for the "bake a stopped object into the background" trick in a named period game, so that part rests on inference.

### Cited Findings
- On the Spectrum, moving a sprite one character column is just "add one to the screen address". This is "straightforward, but it is not smooth" — [oldmachinery, ZX sprites (2014)](https://oldmachinery.blogspot.com/2014/04/zx-sprites.html)
- Smooth horizontal motion uses pre-shifted variants: "each graphic needs 8 shifted variants", and a shifted 16x16 sprite is "in reality a 24x16 sprite". The downside is that "it takes more memory" — [oldmachinery](https://oldmachinery.blogspot.com/2014/04/zx-sprites.html)
- Masked drawing: each sprite has a mask that clears its area before the graphic is ORed in. Mask and graphic bytes are interleaved so one Z80 POP loads both. Fully unrolled per-row code would exceed 20 KB, so the author used self-modifying code instead — [oldmachinery](https://oldmachinery.blogspot.com/2014/04/zx-sprites.html)
- Throughput in that demo: "eight 16x16 masked, freely positioned, flicker-free, smoothly moving sprites". The fast copy alone could move 12 to the screen, but not more than 8 once buffer work and clearing were counted — [oldmachinery](https://oldmachinery.blogspot.com/2014/04/zx-sprites.html)
- XOR drawing makes erasing free, because you erase by drawing again. It was used in Atic Atac and partly caused colour clash. Some games, such as Dan Dare, skip masks and backgrounds altogether — [oldmachinery (search snippet and page)](https://oldmachinery.blogspot.com/2014/04/zx-sprites.html); [Ralph Becket, Faster sprites on ZX Spectrum](https://ralphbecket.blogspot.com/2015/09/faster-sprites-on-zx-spectrum.html)
- An AtariAge user calls the C64 Gauntlet port "a simple character-based Gauntlet". Another C64 programmer in the same thread instead describes the enemies as multiplexed sprites. The sources conflict and neither is authoritative — [AtariAge: large number of sprites i.e. Gauntlet](https://atariage.com/forums/topic/59430-large-number-of-sprites-ie-gauntlet/)
- The MEGA65 chipset reference recommends NCM (16-colour, 16x8-pixel) characters for multiple layers, because NCM "uses fewer bus cycles per pixel than FCM" and needs fewer characters to cover the screen — [MEGA65 Chipset Reference (PDF)](https://files.mega65.org/files/m/mega65-chipset-reference_cnFcKB.pdf)

### Inferences
- (Inference) Bubbles drift slowly and often stop at the ceiling. A cheap scheme follows from that: draw a bubble that has come to rest as background characters, and keep only moving bubbles as sprites or RRB objects. That frees the per-line budget. I found no cited period game documenting this for bubbles.
- (Inference) Pre-shifting matters much less on the MEGA65. GOTOX gives pixel-exact X, and the FCM Y-offset or NCM row mask gives sub-character Y without copying data (see the last section).

### Gaps
- I could not find a primary source, such as a disassembly or developer interview, documenting the "object becomes background when it stops" technique in a named 8-bit game.
- An emix8.org Bubble Bobble disassembly analysis showed up in search but would not load (bot-check page). It would be a high-value primary source: https://emix8.org/bbdisasm

## Software sprites with background save/restore, dirty rectangles, and pre-shifted graphics

### Takeaway
There were three main ways to remove a software sprite: XOR redraw, save and restore the background under it, or restore from a clean background buffer or redraw the whole background. Each trades CPU time and memory against overlap correctness. The MEGA65 documentation says RRB objects avoid this whole undraw/redraw problem.

### Cited Findings
- The traditional approach removes the sprite's previous image before drawing the new one. Overlapping sprites make erasing harder in the save/restore model — [Ralph Becket](https://ralphbecket.blogspot.com/2015/09/faster-sprites-on-zx-spectrum.html)
- One analysed game repaints the background each frame, so there is "no need for erasing sprites at previous location" — [emix8.org disassembly (search snippet only; page not fetchable)](https://www.emix8.org/ggdisasm)
- An emix8 analysis of Bubble Bobble says its erase routine "uses AND-mask when replacing the background". It also says it keeps a separate background-only buffer as the source for erasing sprites. The platform of that port was not visible in the snippet — [emix8.org bbdisasm (search snippet only)](https://emix8.org/bbdisasm)
- The MEGA65 chipset reference describes RRB pseudo-sprites as "similar to 'bobs' on Amiga computers, but with the advantage that they are rendered in the display pipeline, and thus do not need to be un-drawn and re-drawn to animate them" — [MEGA65 Chipset Reference](https://files.mega65.org/files/m/mega65-chipset-reference_cnFcKB.pdf)

### Inferences
- (Inference) On the MEGA65 there is no reason to use save/restore or dirty rectangles for bubbles. Rebuilding each row's RRB draw list replaces erasing. The cost moves from pixel copying to writing a few bytes of screen and colour RAM per object per character row.

### Gaps
- I found no cycle-counted comparison of XOR, save/restore and full redraw for a named game. I also found no explicit "dirty rectangle" write-up for an 8-bit title.

## Sprite multiplexing with raster interrupts, and flicker/rotation for per-line limits

### Takeaway
C64 multiplexers sort virtual sprites by Y once per frame and reassign the 8 hardware sprites in raster IRQs further down the screen. 32 virtual sprites is the common benchmark. A line still never shows more than 8, so extra sprites are dropped or flickered. On the NES, games rotate OAM order each frame ("OAM cycling") so that the 8-per-scanline limit appears as flicker instead of permanently invisible sprites.

### Cited Findings
- "It is simply impossible to display more than 8 sprites on the same raster line" on the C64. Multiplexers sort by increasing Y, map virtual sprites to physical ones round-robin, and reject sprites when a Y-gap "less than 21 pixels" would put a 9th on a line. Without rejection, "some definitely ugly graphics will appear" — [Cadaver, Sprite multiplexing](https://cadaver.github.io/rants/sprite.html)
- Raster IRQs must fire before the sprite's Y. Rewriting all 8 sprites can need IRQs "about 10-12 lines before". Interrupts closer than "3 lines" should be chained directly, because 2 lines sometimes caused flicker. The sorted array should be double-buffered so the IRQs display one half while the main loop sorts the other — [Cadaver](https://cadaver.github.io/rants/sprite.html)
- Sort choices: bubble sort and next-lowest search grow with N². A bucket sort by Y/8 is fast but gives the wrong order. Radix sort is correct and linear but memory-hungry. Continuous insertion sort ("Ocean sort", also used in SWIV) is fast on average because order rarely changes, but "on some frame it'll eat a lot of time" — [Cadaver](https://cadaver.github.io/rants/sprite.html)
- Games using general multiplexing: Green Beret, Ghosts'n Goblins, Midnight Resistance, Turrican, Dragon Breed, SWIV, Metal Warrior 1-4, BOFH — [Cadaver](https://cadaver.github.io/rants/sprite.html)
- Zone split, the simpler method: before each row of a large object, an IRQ rewrites Y, frame and colour for a group of sprites. Example: a 20-sprite boss in 5 per row by 4 rows. The cost is losing free use of those sprites — [Cadaver](https://cadaver.github.io/rants/sprite.html)
- Field sort is a 32-actor bucket sort over 220 Y-positions, using a self-modifying field of `iny` opcodes. It costs 2 cycles per empty bucket (440 base) and 2208 cycles worst case for 32 actors, in under 2 kB. Christopher Jam's version reached 1972 cycles but used over 32 kB — [Linus Åkesson, Field sort](https://hd0.linusakesson.net/programming/fieldsort/index.php)
- Mike Dailly lists three multiplexing styles:
  1. Fixed zones with fixed IRQs, as in Delta (5 zones).
  2. Bands, as in Ballistix and Morpheus. A sprite straddling two bands is set up twice.
  3. Dynamic IRQs placed at runtime from the sorted positions, as in Blood Money and Armalyte. Armalyte sorted using the stack; Blood Money used a slower insertion sort.

  Blood Money ran at "3 frames" per update before the multiplexer was fixed, and nearly reached 2 after — [Mike Dailly, Rasters, sprites and multiplexing](https://lemmings.info/rasters-sprites-and-multiplexing/)
- NES: the PPU evaluates OAM in order and stops after 8 sprites on a scanline. The sprite-overflow flag is unreliable because of a hardware bug — [nesdev wiki: PPU sprite evaluation](https://www.nesdev.org/wiki/PPU_sprite_evaluation); [nesdev wiki: Errata](https://nesdev.org/wiki/Errata)
- "OAM cycling" reorders sprite data each frame so sprites take turns being among the first 8 on a line. Scanlines with more than 8 objects then flicker instead of losing the same sprite every frame — [NESdev forum archive](https://archive.nes.science/nesdev-forums/f3/t5686.xhtml)
- NES games also use the limit on purpose. Eight blank sprites at one Y mask other sprites on those lines, as in Castlevania II swamps, Zelda dungeon edges and Ninja Gaiden cutscenes — [nesdev wiki: Sprite overflow games](https://www.nesdev.org/wiki/Sprite_overflow_games)
- On the MEGA65, the VIC-IV uses a ring buffer for each sprite's data, "so a sprite can be displayed multiple times per raster line, thus potentially allowing for horizontal multiplexing" — [MEGA65 Chipset Reference](https://files.mega65.org/files/m/mega65-chipset-reference_cnFcKB.pdf)

### Inferences
- (Inference) The nearly-sorted insertion sort and the double-buffered sort array carry over directly to building MEGA65 RRB draw lists. Objects there must also be grouped by character row, then ordered back to front.
- (Inference) The NES rotation idea carries over to the RRB too. If a row's draw list would exceed the per-line budget, rotate which bubbles are emitted on that row each frame. That gives flicker instead of a truncated right-hand side of the line (RRB overruns cut off the line; see the last section).

### Gaps
- I could not fetch the codebase64 multiplexer pages directly, so their specific figures are not included.
- I found no authoritative "maximum virtual sprites" figure for C64 multiplexers. Cadaver gives no fixed maximum, and 32 is the benchmark used in sort comparisons.

## Reduced or staggered update rates, and simplified physics for many objects

### Takeaway
Primary period sources on staggered updates are thin. The best documented example is the C64 Blood Money running its sprite update at 3, then about 2, frames while the player moved every VBlank. The general practice of spreading AI or physics updates across frames is described mostly in modern hobbyist discussion.

### Cited Findings
- Blood Money moved the player every VBlank, which felt smoother than its slower overall sprite update rate (3 frames, later nearly 2) — [Mike Dailly](https://lemmings.info/rasters-sprites-and-multiplexing/)
- Ocean-style insertion sorting uses temporal coherence: sprites rarely change order between frames, so the sort usually does little work — [Cadaver](https://cadaver.github.io/rants/sprite.html); [Linus Åkesson](https://hd0.linusakesson.net/programming/fieldsort/index.php)
- Developers recommend lowering AI update frequency and offsetting updates so they don't all spike on the same tick. Round-robin tied to list length scales badly as the object count grows; fixed groups avoid that — [Unity discussion: realtime roguelike timestep](https://discussions.unity.com/t/managing-realtime-roguelike-timestep/654110); search-result summaries of [Godot forum](https://forum.godotengine.org/t/general-coding-strategy-to-not-cap-out-cpu-while-ai-moves-enemies-loop-question/5267) and [Phaser forum](https://phaser.discourse.group/t/how-to-efficiently-deal-with-enemies-ai/8086) (modern, secondary)
- An AtariAge blog post on fractional movement describes round-robin movement logic that updates each enemy and the player on a timed cycle — [AtariAge blog: fractional movement (search snippet)](https://atariage.com/forums/blogs/entry/1449-fractional-movement/)
- The NES-era advice for AI is to keep the formula from taking up too many CPU cycles — [Super Tilt Bro devlog: AI on the NES](https://sgadrat.itch.io/super-tilt-bro/devlog/6252/artificial-intelligence-on-the-nes)

### Inferences
- (Inference) For a bubble swarm, the obvious split is this:
  - Every frame: update position from a fixed velocity using fixed-point adds, and rebuild the RRB draw list.
  - Staggered across N frames: wall and ceiling collisions, bubble-to-bubble pushing, and lifetime timers, on a subset each frame (for example index mod 4).
- (Inference) Bubble-to-bubble interaction is O(N²) if done naively. Bucketing bubbles by character row, which the RRB already requires, gives a cheap broad phase.

### Gaps
- I found no primary-source documentation, such as a disassembly or postmortem, of a named 8-bit game that staggers enemy or physics updates across frames. How the arcade or 8-bit Bubble Bobble ports handle large bubble counts was not established.

## MEGA65 specifics: VIC-IV RRB/GOTOX capacity, documented limits, and examples

### Takeaway
The official chipset reference gives no per-line character or object count: "The amount of material is limited only by the raster time of the VIC-IV. Some experimentation will be required". The best documented empirical figure is Dan Sanderson's stress test. With 12 rows of 20 NCM (16 px wide) characters as full-width layers, real hardware handled 11 layers and missed lines at 12; Xemu handled 12 and failed at 13. DBLRR doubles RRB time at the cost of V200 resolution. A third-party FPGA fork gives per-token cycle costs, but its figures conflict with Sanderson's results and should not be taken as stock-hardware numbers.

### Cited Findings
- **What RRB is.** With the GOTO bit set in Super-Extended Attribute Mode (SEAM), a two-byte screen/colour entry draws nothing. Instead it moves the raster render position to a 10-bit pixel X. This lets text or graphics be drawn over other text or graphics, as pseudo-sprites — [MEGA65 Chipset Reference](https://files.mega65.org/files/m/mega65-chipset-reference_cnFcKB.pdf)
- **Official capacity statement.** "The amount of material is limited only by the raster time of the VIC-IV. Some experimentation will be required to determine how much can be achieved in PAL and NTSC modes." — [MEGA65 Chipset Reference](https://files.mega65.org/files/m/mega65-chipset-reference_cnFcKB.pdf)
- **Vertical placement without copying.** With GOTO set and FCM, the width-reduction bits act as a Y offset into the character data. With NCM, the second colour byte is an 8-bit row mask, one bit per pixel row. Together these give "free vertical placement of soft sprites, without needing to copy the character data" — [MEGA65 Chipset Reference](https://files.mega65.org/files/m/mega65-chipset-reference_cnFcKB.pdf); [Dan Sanderson, Pixie Power](https://dansanderson.com/mega65/pixies/) gives the Y offset range as "-7 to +7"
- **Termination rule.** The VIC-IV shows only data up to and including the last drawn character. Every row must end with a GOTOX to the right edge followed by a character — [MEGA65 Chipset Reference](https://files.mega65.org/files/m/mega65-chipset-reference_cnFcKB.pdf); [Dan Sanderson, RRB article](https://dansanderson.com/mega65/raster-rewrite-buffer/); [RetroCogs, VIC-IV graphics: using RRB for pixies (2025)](https://retrocogs.mega65.com/2025/08/14/vic-iv-graphics-using-rrb-for-pixies/)
- **Related registers:**
  - DBLRR: "When set, the Raster Rewrite Buffer is only updated every 2nd raster line, limiting resolution to V200, but allowing more cycles for Raster-Rewrite actions."
  - NORRDEL: "When clear, raster rewrite double buffering is used."

  [MEGA65 Chipset Reference](https://files.mega65.org/files/m/mega65-chipset-reference_cnFcKB.pdf)
- **Display timing.**
  - The VIC-IV has an 81 MHz internal pixel clock, and output runs at 27 MHz (1/3 of it).
  - A PAL frame is 624 physical rasters of 864 output-pixel ticks.
  - Each logical (VIC-II-style) raster is 2 physical rasters: 63 ~1 MHz cycles for PAL (312 logical lines) and 65 for NTSC (263 logical lines).
  - The VIC-IV has no native bad lines, because it has its own memory buses.

  [MEGA65 Chipset Reference](https://files.mega65.org/files/m/mega65-chipset-reference_cnFcKB.pdf)
- **Row sizing rule.** "maximum characters + GOTOX per row = CHRCOUNT <= LINESTEP/2" (each entry is 2 bytes in SEAM). CHRCOUNT is 10 bits ($D05E plus $D063 bits 4-5).
  - Worked example: 20 tiles x 3 layers, so LINESTEP 124 (128 suggested for easy addressing) and CHRCOUNT 62. That is about 2,976 bytes each of screen and colour RAM for 12 rows.

  [Dan Sanderson, Roguecraft DX: Introducing the RRB](https://dansanderson.com/mega65/raster-rewrite-buffer/)
- **Empirical stress test, the key documented number.** Setup: 320x200, 12 rows of 20 NCM characters (16 px each), full-width layers.
  - Real MEGA65: 11 layers, with lines missed at 12.
  - Xemu: 12 layers, failing at 13.
  - Caveat from the article: results vary with FCM, other modes and resolutions.

  The test program is DISPLAY5 on `tactical.d81`, where `LC = 11` sets the layer count — [Dan Sanderson, RRB article](https://dansanderson.com/mega65/raster-rewrite-buffer/); [Dan Sanderson, Pixie Power](https://dansanderson.com/mega65/pixies/)
- **Doubling RRB time in 200-line mode.** Set V400 ($D031.3), CHRYSCL ($D05B) = 0 and DBLRR ($D051.6), and TEXTYPOS ($D04E) = 102, which must be even with DBLRR — [Dan Sanderson, RRB article](https://dansanderson.com/mega65/raster-rewrite-buffer/)
- **Recommended layout for many objects.** Each row holds the full background, then a "draw list" of GOTOX + character pairs for foreground objects, ordered back to front. Only the draw list is rebuilt when objects move. The example row is 40 bytes of background, a 2-byte GOTOX, then 42 more bytes — [Dan Sanderson, RRB article](https://dansanderson.com/mega65/raster-rewrite-buffer/); [Dan Sanderson, Pixie Power](https://dansanderson.com/mega65/pixies/)
- **Other pitfalls.**
  - Fill unused row memory with dummy characters so stray GOTOX entries don't corrupt the row.
  - Clear HOTREG ($D05D.7) to avoid register recalculation side effects.
  - Real hardware and Xemu differ in edge cases. REALHW ($D60F.5) can be read to detect which one is running.

  [Dan Sanderson, RRB article](https://dansanderson.com/mega65/raster-rewrite-buffer/)
- **Line buffer width.** RetroCogs describes the RRB as "a 2048 pixel wide single line buffer" — [RetroCogs (2025)](https://retrocogs.mega65.com/2025/08/14/vic-iv-graphics-using-rrb-for-pixies/). It does not give a per-line time budget.
- **Third-party cycle figures, treat with caution.** A hobbyist MEGA65 FPGA project on a different board says:
  - The VIC-IV paints into the line buffer at "one pixel per pixel clock; what does not fit into the line time is cut off".
  - Before its own optimisations: "8 cycles for a plain glyph … 16 for a full-colour glyph".
  - With its added "fast token path" (XCTL bit 7, which appears to be the project's own extension): FCM 16 -> 4 cycles, NCM 8, plain 6, GOTOX about 9. It claims "128 one-glyph objects on one row fit a raster line and 255 fit DBLRR's two lines".
  - Its stress script "finds 2 full-width 40-character layers at 1 px/cycle".

  The page does not say whether these numbers were measured or derived — [beniacm/mega65-k325t-cm4](https://github.com/beniacm/mega65-k325t-cm4). Its "2 full-width layers" figure appears to conflict with Sanderson's 11 NCM layers on stock hardware. The modes differ, but the gap is large, so do not use these numbers for stock MEGA65 planning.
- **Old emulator discrepancy, resolved.** Xemu issue #253 (May 2021) reported that Xemu "seems to crop lines at 20 chars" for RRB compared with hardware. It was closed in June 2021 as a duplicate of a fixed issue (#265) — [xemu issue #253](https://github.com/lgblgblgb/xemu/issues/253)
- **Examples using RRB.**
  - Roguecraft DX (Badger Punch Games; MEGA65 version by RetroCogs) uses RRB for "animated, colorful foreground figures on a colorful background". No object count is given.
  - Snowflake (geehaf) is "a simple demo using a technique based on the raster rewrite buffer".
  - Dan Sanderson's in-progress Tactical Strike is the worked example.
  - Sanderson says RRB can support "hundreds of fast-moving graphics objects, particle systems, and multiple parallax background layers". That is a capability claim, not a benchmark.

  [Dan Sanderson, RRB article](https://dansanderson.com/mega65/raster-rewrite-buffer/)
- RetroCogs' GameShell65, an assembly framework, includes pixie abstractions and a `pixieReadme.md` — [Dan Sanderson, Pixie Power](https://dansanderson.com/mega65/pixies/). I could not locate the repository by search.
- The MEGA65 also has 8 hardware sprites, which can use a separate 256-colour palette and draw data from anywhere in memory (SPRPTR16) — [MEGA65 Chipset Reference](https://files.mega65.org/files/m/mega65-chipset-reference_cnFcKB.pdf)

### Inferences
- (Inference, rough budget) On stock hardware, about 11 full-width NCM layers of 20 chars is about 220 NCM characters per character row (11 x 20, plus about 11 GOTOX tokens). A one-character (16x8 NCM) bubble costs one GOTOX + one char. Most bubbles span 2 character rows unless aligned, so each costs entries in two rows.
  - If 2 layers are spent on a full background plus padding, roughly 9 layers' worth of tokens remains, about 90 GOTOX+char pairs per row (half the tokens are GOTOX). That suggests a few dozen 16 px bubbles per character row.
  - This assumes GOTOX costs about the same as a character. Nothing documents that for stock hardware, and Sanderson's test used few GOTOX tokens, so the real number could be well below this.
  - Benchmark on hardware with a GOTOX-heavy draw list before relying on it.
- (Inference) DBLRR roughly doubles the budget per row, and V200 matches a 320x200 game. It is the natural first lever if bubble density overruns a row.
- (Inference) Overrun behaviour is truncation: content past the time budget is cut off on the right. So emit the most important objects first (the player, enemies) and decorative bubbles last, or rotate bubble order NES-style so overflow flickers instead of permanently hiding the same bubbles. But RRB order is also z-order (back to front), so this conflicts with layering, and the trade-off needs design.
- (Inference) The 8 hardware sprites, which use a separate palette and can be multiplexed C64-style, are a good fit for the player characters. That leaves RRB for the swarm.

### Gaps
- The official documentation gives no figure for RRB time per raster line in cycles or pixels, or for the cost of a GOTOX token. It explicitly defers to experimentation.
- I found no documented benchmark of "N objects per line" on stock hardware, nor any FCM or 640-wide stress test figures.
- I found no MEGA65 forum, Discord or Filehost write-up with object counts for a specific game or demo. RetroCogs' GameShell65 pixie README could not be located.
