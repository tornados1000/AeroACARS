# Discord draft — AeroACARS v0.20.0

> Draft. NOT yet posted. Channel: probably #announcements.
> There are placeholders in two spots: the name of the reporter (if you want to name them)
> and the channel reference. Otherwise the text can go out as-is.

---

**✈️ AeroACARS v0.20.0 is here — "one landing, one score"**

One of you reported that the same touchdown shows **two different sink rates**: the landing card said **−206 fpm**, the ACARS log line right next to it **−233 fpm**. Thanks for that — the report uncovered far more than that single value.

**What was really going on**

It wasn't a calculation error but a pattern: **almost every number in the landing tab was computed independently in two or three places** — once in the app, once again for the log, sometimes a third time for the live map. Which number you saw depended on where you looked. That's also why earlier corrections always felt like patchwork: each time we cured a symptom, never the cause.

**What we did**

The app now scores your landing **exactly once**, freezes the result, and every display shows only that one result — landing tab, ACARS log, PIREP, live map, PDF.

While cleaning up, five more contradictions turned up that **nobody had reported**:

• **G-force was shown twice in the tab** — the tile showed the raw value, the bar next to it the smoothed one.
• **Grade and class contradicted each other** — the PIREP had "A (smooth) — 92/100" next to "A+ (SMOOTH, 100/100)". Also the boundaries didn't line up: 47 points yielded "F (firm)", 88 points "A (acceptable)".
• **Runway utilization was computed three times** — and the runway diagram even drew the runway **too long** with a displaced threshold.
• **Approach stability counted your deliberate turns during the approach as "unsteadiness"** — depending on which display you looked at.
• **The "best value"** was sorted on a raw value and thereby crowned the wrong best landing on old flights.

And the famous **109 kg** from the loadsheet: the tile showed the **SimBrief plan**, the log the **actual measurement**. Not a bug — it's simply the taxi fuel. The tile now says "plan" next to it.

**What this means for you**

• **Your points may shift slightly.** Approach stability now uses the clean value, and the class words now sit on the grade boundaries. We didn't touch the formulas themselves.
• **Already-flown landings are not recomputed.** They keep their numbers.
• For **old** landings the category word may change in the web dashboard: it now describes the overall score (incl. approach, runway, fuel) instead of just the touchdown. A buttery landing after a shaky approach previously read as "SMOOTH · 62".

**So it doesn't come back**

The rule "there is only one score" is now no longer just a comment in the code but is **enforced**: a test fails as soon as a display starts computing on its own again. That's exactly where it broke last time — the rule was there, and yet someone overrode it.

**Update:** start AeroACARS, the update will be offered to you.

*And please keep the reports coming. A screenshot with two numbers on it found more here than any test run.*
