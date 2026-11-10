"""
v0.9.0 (#Discord-RPC) — Discord-Rich-Presence-Asset-Generator (v2, designed).

Generiert 5 Discord-Assets als 1024x1024 PNGs:
  - aeroacars_logo  : Brand-Logo auf Cyan→Gelb-Gradient (matched die Source-Farben)
  - sim_msfs2024    : Top-Down-Jet, Microsoft-Cobalt, Stars-im-Hintergrund (= MSFS "World")
  - sim_msfs2020    : Top-Down-Jet, Microsoft-Indigo, Wolken-Hintergrund
  - sim_xplane12    : Top-Down-Jet, X-Plane-Smaragd, X-Roundel-Hintergrund
  - sim_xplane11    : Top-Down-Jet, X-Plane-Wald, X-Roundel-Hintergrund

Spec: docs/spec/v0.9.0-discord-rich-presence.md (LE4 Asset-Layout)

Design-Prinzipien:
  - Aviation-themed Brand-Identitaet: jeder Sim hat ein top-down-Aircraft-Silhouette
    plus ein Sim-spezifisches Hintergrund-Motiv (Stars/Wolken/X)
  - Lesbar bei 30x30 px Discord-small_image: starke Silhouette > Filigran-Detail
  - Eigene Farben + abstrakte Motive — keine geklauten Logos
  - Linear-Gradient + Inner-Glow + Drop-Shadow fuer Tiefe

Run:
  python docs/discord-assets/generate.py
"""

from __future__ import annotations
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter, ImageFont
import math
import random

OUT = Path(__file__).parent
SIZE = 1024
COVER_W, COVER_H = 1024, 576   # Discord Rich-Presence Einladungs-Cover (16:9)
SOURCE_LOGO = OUT.parent.parent / "client" / "src-tauri" / "icons" / "icon.png"


# ─── Utilities ────────────────────────────────────────────────────────────

def find_font(size: int, bold: bool = True) -> ImageFont.FreeTypeFont:
    """Bevorzugt einen verfuegbaren Sans-Serif-Bold-Font."""
    candidates_bold = [
        "C:/Windows/Fonts/segoeuib.ttf",   # Segoe UI Bold
        "C:/Windows/Fonts/arialbd.ttf",
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    ]
    candidates_regular = [
        "C:/Windows/Fonts/segoeui.ttf",
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    for path in (candidates_bold if bold else candidates_regular):
        if Path(path).exists():
            try:
                return ImageFont.truetype(path, size)
            except Exception:
                continue
    return ImageFont.load_default()


def linear_gradient(size: int, c1: tuple, c2: tuple, angle_deg: float = 135.0) -> Image.Image:
    """Quadratisches Lineares Gradient. angle=135 = top-left → bottom-right."""
    return linear_gradient_rect(size, size, c1, c2, angle_deg)


def linear_gradient_rect(w: int, h: int, c1: tuple, c2: tuple, angle_deg: float = 90.0) -> Image.Image:
    """Lineares Gradient fuer beliebige Rechtecke. angle=90 = top→bottom, 0 = left→right."""
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    px = img.load()
    rad = math.radians(angle_deg)
    dx, dy = math.cos(rad), math.sin(rad)
    diag = abs(dx) * w + abs(dy) * h
    for y in range(h):
        for x in range(w):
            t = ((x * dx) + (y * dy)) / diag
            t = max(0.0, min(1.0, t + 0.5))
            r = int(c1[0] + (c2[0] - c1[0]) * t)
            g = int(c1[1] + (c2[1] - c1[1]) * t)
            b = int(c1[2] + (c2[2] - c1[2]) * t)
            a = int(c1[3] + (c2[3] - c1[3]) * t) if len(c1) == 4 else 255
            px[x, y] = (r, g, b, a)
    return img


def multi_stop_vertical(w: int, h: int, stops: list[tuple[float, tuple]]) -> Image.Image:
    """Mehr-Stop-Vertikalgradient. `stops` ist [(0.0, rgb), (0.4, rgb), (1.0, rgb), ...].
    Stops MUESSEN sortiert sein nach Position 0..1.
    """
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    px = img.load()
    for y in range(h):
        t = y / max(1, h - 1)
        # find segment
        for i in range(len(stops) - 1):
            t0, c0 = stops[i]
            t1, c1 = stops[i + 1]
            if t0 <= t <= t1:
                local = (t - t0) / max(1e-9, t1 - t0)
                r = int(c0[0] + (c1[0] - c0[0]) * local)
                g = int(c0[1] + (c1[1] - c0[1]) * local)
                b = int(c0[2] + (c1[2] - c0[2]) * local)
                color = (r, g, b, 255)
                break
        else:
            color = (*stops[-1][1], 255)
        for x in range(w):
            px[x, y] = color
    return img


def rounded_mask(size: int, radius: int) -> Image.Image:
    """Alpha-Mask fuer abgerundetes Quadrat."""
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    return m


def draw_text_with_shadow(draw, xy, text, font, fill, shadow=(0, 0, 0, 140), offset=6):
    """Text mit weichem Schatten unten-rechts."""
    x, y = xy
    draw.text((x + offset, y + offset), text, font=font, fill=shadow)
    draw.text((x, y), text, font=font, fill=fill)


def centered_text_box(draw, text, font, cx, cy):
    """Gibt (x, y) zurueck sodass text zentriert um (cx, cy) zu liegen kommt."""
    bbox = draw.textbbox((0, 0), text, font=font)
    w = bbox[2] - bbox[0]
    h = bbox[3] - bbox[1]
    return (cx - w // 2 - bbox[0], cy - h // 2 - bbox[1])


# ─── Top-Down Aircraft-Silhouette ─────────────────────────────────────────
# Ein generischer Twin-Engine Airliner von oben — A320/B737-Anmutung.
# Punkte sind in 1024x1024 zentriert + skaliert; alle Werte als float damit
# wir runter/hoch skalieren koennen ohne Pixel-Crunch.

JET_POLY = [
    # Nase
    (512, 80),
    # Cockpit-Schultern
    (548, 140), (560, 200),
    # Fuselage rechts vor Wing
    (570, 380),
    # Wing rechts oben (Vorderkante)
    (590, 440),
    (940, 580),
    # Wing rechts unten (Hinterkante)
    (610, 600),
    (590, 620),
    # Fuselage rechts nach Wing
    (570, 760),
    # Heck-Stabilizer rechts (Vorderkante)
    (610, 800),
    (760, 870),
    # Heck-Stabilizer rechts (Hinterkante)
    (610, 900),
    # Heck Mitte rechts
    (560, 920),
    (540, 935),
    # Heck-Mitte
    (512, 940),
    # Mirror linke Seite
    (484, 935),
    (464, 920),
    (414, 900),
    (264, 870),
    (414, 800),
    (454, 760),
    (434, 620),
    (414, 600),
    (84, 580),
    (434, 440),
    (454, 380),
    (464, 200),
    (476, 140),
]


def draw_jet(canvas: Image.Image, color: tuple, x_offset: int = 0, y_offset: int = 0,
             scale: float = 1.0, opacity: int = 230) -> None:
    """Zeichnet die Jet-Silhouette mit Tiefen-Shadow auf das Canvas."""
    # Shadow-Layer
    sh = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    sd = ImageDraw.Draw(sh)
    poly = [(x_offset + 512 + (px - 512) * scale, y_offset + 512 + (py - 512) * scale)
            for px, py in JET_POLY]
    sd.polygon([(p[0] + 12, p[1] + 16) for p in poly], fill=(0, 0, 0, 120))
    sh = sh.filter(ImageFilter.GaussianBlur(radius=14))
    canvas.alpha_composite(sh)

    # Hauptpolygon
    layer = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    ld = ImageDraw.Draw(layer)
    fill = (color[0], color[1], color[2], opacity)
    ld.polygon(poly, fill=fill)
    # Subtile innere Highlight-Linie (= „Glanzlicht" an Vorderkanten)
    highlight = (255, 255, 255, 60)
    # Nase + Cockpit
    ld.line([(484, 200), (512, 80), (540, 200)],
            fill=highlight, width=4)
    canvas.alpha_composite(layer)


# ─── Hintergrund-Motive ───────────────────────────────────────────────────

def add_starfield(canvas: Image.Image, count: int = 90, max_size: int = 4, seed: int = 1) -> None:
    """Verstreute kleine Sterne — MSFS-„from-space"-Vibe."""
    rng = random.Random(seed)
    overlay = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(overlay)
    for _ in range(count):
        x = rng.randint(20, SIZE - 20)
        y = rng.randint(20, SIZE - 20)
        r = rng.choice([1, 1, 1, 2, 2, 3, max_size])
        alpha = rng.randint(120, 220)
        d.ellipse([x - r, y - r, x + r, y + r], fill=(255, 255, 255, alpha))
    overlay = overlay.filter(ImageFilter.GaussianBlur(radius=0.6))
    canvas.alpha_composite(overlay)


def add_cloud_arcs(canvas: Image.Image, color: tuple, seed: int = 2) -> None:
    """Weiche horizontale Wolken-Schwaden — MSFS-2020-Vibe (wetter-driven)."""
    rng = random.Random(seed)
    overlay = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(overlay)
    for i in range(6):
        y = 120 + i * 145 + rng.randint(-20, 20)
        w = rng.randint(380, 720)
        x = rng.randint(0, SIZE - w)
        h = rng.randint(40, 80)
        d.ellipse([x, y, x + w, y + h], fill=(*color, rng.randint(35, 70)))
    overlay = overlay.filter(ImageFilter.GaussianBlur(radius=18))
    canvas.alpha_composite(overlay)


def add_x_roundel(canvas: Image.Image, color: tuple) -> None:
    """X-Roundel im Hintergrund — X-Plane-Visual-Cue."""
    overlay = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(overlay)
    # Konzentrische Kreise (RAF-Roundel-Stil)
    for r, a in [(440, 28), (360, 36), (280, 44), (200, 52)]:
        d.ellipse([512 - r, 512 - r, 512 + r, 512 + r], outline=(*color, a + 30), width=6)
    # Großes „X" mittig
    bx = 200
    d.line([(512 - bx, 512 - bx), (512 + bx, 512 + bx)], fill=(*color, 90), width=22)
    d.line([(512 + bx, 512 - bx), (512 - bx, 512 + bx)], fill=(*color, 90), width=22)
    overlay = overlay.filter(ImageFilter.GaussianBlur(radius=2))
    canvas.alpha_composite(overlay)


# ─── Sim-Badge ────────────────────────────────────────────────────────────

def make_sim_badge(
    asset_name: str,
    title: str,
    year: str,
    bg_top: tuple,
    bg_bottom: tuple,
    accent: tuple,
    bg_motif: str,  # "stars" | "clouds" | "x_roundel"
) -> None:
    """Erstellt ein vollwertiges Sim-Badge: Gradient-Hintergrund, Motif,
    Jet-Silhouette + Title + Year-Banner.
    """
    # 1. Hintergrund-Gradient
    canvas = linear_gradient(SIZE, (*bg_top, 255), (*bg_bottom, 255), angle_deg=135)

    # 2. Vignette (= leicht dunkler an den Raendern fuer Tiefe)
    vignette = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    vd = ImageDraw.Draw(vignette)
    for i, a in enumerate([0, 0, 10, 20, 40, 70]):
        vd.rectangle([i * 6, i * 6, SIZE - i * 6, SIZE - i * 6], outline=(0, 0, 0, a), width=6)
    canvas.alpha_composite(vignette)

    # 3. Hintergrund-Motiv (Sim-spezifisch)
    if bg_motif == "stars":
        add_starfield(canvas, count=110, seed=hash(asset_name) & 0xFFFF)
    elif bg_motif == "clouds":
        add_cloud_arcs(canvas, color=(255, 255, 255), seed=hash(asset_name) & 0xFFFF)
    elif bg_motif == "x_roundel":
        add_x_roundel(canvas, color=accent)

    # 4. Aircraft-Silhouette (weiss, leicht transparent)
    draw_jet(canvas, color=(255, 255, 255), scale=0.62, x_offset=0, y_offset=-30, opacity=215)

    # 5. Year-Banner unten — Sim-Name oben
    draw = ImageDraw.Draw(canvas)
    font_title = find_font(int(SIZE * 0.115))
    font_year = find_font(int(SIZE * 0.16))

    # Sim-Title-Plakette ganz oben (kleine Pille)
    title_pad_x = 28
    title_pad_y = 14
    bbox = draw.textbbox((0, 0), title, font=font_title)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    pill_w = tw + title_pad_x * 2
    pill_h = th + title_pad_y * 2
    pill_x = (SIZE - pill_w) // 2
    pill_y = 60
    draw.rounded_rectangle(
        [pill_x, pill_y, pill_x + pill_w, pill_y + pill_h],
        radius=pill_h // 2,
        fill=(255, 255, 255, 230),
    )
    draw.text(
        (pill_x + title_pad_x - bbox[0], pill_y + title_pad_y - bbox[1]),
        title, font=font_title, fill=accent
    )

    # Year als gross-zentriertes Element unten (Tiefe + Schatten)
    yw_bbox = draw.textbbox((0, 0), year, font=font_year)
    yw_w = yw_bbox[2] - yw_bbox[0]
    yw_h = yw_bbox[3] - yw_bbox[1]
    yw_x = (SIZE - yw_w) // 2 - yw_bbox[0]
    yw_y = SIZE - yw_h - 130 - yw_bbox[1]
    # Schatten
    draw.text((yw_x + 6, yw_y + 8), year, font=font_year, fill=(0, 0, 0, 150))
    draw.text((yw_x, yw_y), year, font=font_year, fill=(255, 255, 255, 250))

    # 6. Auf abgerundetes Quadrat zuschneiden (Mask)
    mask = rounded_mask(SIZE, radius=int(SIZE * 0.12))
    final = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    final.paste(canvas, (0, 0), mask)

    # 7. Subtiler Rand
    border = ImageDraw.Draw(final)
    border.rounded_rectangle(
        [3, 3, SIZE - 4, SIZE - 4],
        radius=int(SIZE * 0.12) - 3,
        outline=(255, 255, 255, 60),
        width=4,
    )

    out = OUT / f"{asset_name}.png"
    final.save(out, "PNG", optimize=True)
    print(f"OK {asset_name}: {out.name}  ({SIZE}x{SIZE})")


# ─── AeroACARS-Logo ───────────────────────────────────────────────────────

def make_logo() -> None:
    """aeroacars_logo: source-icon auf dezentem Brand-Gradient (Cyan→Gelb,
    matched die Source-Farben). Kommt auf Discord-Dark-Background gut zur Geltung.
    """
    if not SOURCE_LOGO.exists():
        raise FileNotFoundError(SOURCE_LOGO)
    src = Image.open(SOURCE_LOGO).convert("RGBA").resize((SIZE, SIZE), Image.LANCZOS)

    # Hintergrund: dunkler Gradient + extrem dezente Star-Strukturen
    bg = linear_gradient(SIZE, (15, 28, 48, 255), (32, 14, 38, 255), angle_deg=135)
    add_starfield(bg, count=50, max_size=2, seed=42)

    # Radial-Glow hinter dem Logo (= Cyan + Gelb der Source-Farben)
    glow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    # Cyan-Glow links
    gd.ellipse([100, 100, 760, 760], fill=(28, 188, 220, 90))
    # Gelb-Glow rechts
    gd.ellipse([280, 60, 940, 720], fill=(244, 188, 60, 70))
    glow = glow.filter(ImageFilter.GaussianBlur(radius=80))
    bg.alpha_composite(glow)

    # Logo drauf
    bg.alpha_composite(src)

    # Maskiert auf rounded square
    mask = rounded_mask(SIZE, radius=int(SIZE * 0.12))
    final = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    final.paste(bg, (0, 0), mask)

    # Subtiler Rand
    bd = ImageDraw.Draw(final)
    bd.rounded_rectangle(
        [3, 3, SIZE - 4, SIZE - 4],
        radius=int(SIZE * 0.12) - 3,
        outline=(255, 255, 255, 70),
        width=4,
    )

    out = OUT / "aeroacars_logo.png"
    final.save(out, "PNG", optimize=True)
    print(f"OK aeroacars_logo: {out.name}  ({SIZE}x{SIZE})")


def make_cover() -> None:
    """1024x576 Einladungs-Cover fuer Discord Rich Presence.
    Cinematic sunset-horizon mit Top-Down-Jet-Silhouette + AeroACARS-Brand-Block.
    """
    # 1. Multi-Stop-Sky-Gradient (Space → Twilight → Sunset)
    canvas = multi_stop_vertical(COVER_W, COVER_H, stops=[
        (0.00, (8, 18, 42)),        # Space-Navy ganz oben
        (0.25, (16, 36, 80)),       # Tiefes Twilight
        (0.55, (38, 84, 160)),      # Klares Mid-Blau
        (0.72, (210, 110, 64)),     # Sunset-Orange (Horizon-Glow)
        (0.85, (90, 38, 28)),       # Dunkles Rotbraun (Land unter Horizon)
        (1.00, (24, 14, 18)),       # Fast-Schwarz am Boden
    ])

    # 2. Sterne im oberen Drittel
    rng = random.Random(7)
    star_overlay = Image.new("RGBA", (COVER_W, COVER_H), (0, 0, 0, 0))
    sd = ImageDraw.Draw(star_overlay)
    for _ in range(70):
        x = rng.randint(20, COVER_W - 20)
        y = rng.randint(10, int(COVER_H * 0.35))
        r = rng.choice([1, 1, 1, 2, 2, 3])
        a = rng.randint(140, 230)
        sd.ellipse([x - r, y - r, x + r, y + r], fill=(255, 255, 255, a))
    star_overlay = star_overlay.filter(ImageFilter.GaussianBlur(radius=0.4))
    canvas.alpha_composite(star_overlay)

    # 3. Horizon-Glow (Soft-Light-Band um 70% Hoehe)
    glow = Image.new("RGBA", (COVER_W, COVER_H), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    glow_y = int(COVER_H * 0.70)
    gd.ellipse([COVER_W // 2 - 600, glow_y - 60, COVER_W // 2 + 600, glow_y + 60],
               fill=(255, 200, 130, 160))
    glow = glow.filter(ImageFilter.GaussianBlur(radius=40))
    canvas.alpha_composite(glow)

    # 4. Cloud-Streaks (warm beleuchtet)
    clouds = Image.new("RGBA", (COVER_W, COVER_H), (0, 0, 0, 0))
    cd = ImageDraw.Draw(clouds)
    for i in range(5):
        cy = int(COVER_H * (0.55 + i * 0.04))
        w = rng.randint(300, 600)
        x = rng.randint(-80, COVER_W - 200)
        h = rng.randint(18, 32)
        c = (255, 220 - i * 20, 180 - i * 20, 140 - i * 18)
        cd.ellipse([x, cy, x + w, cy + h], fill=c)
    clouds = clouds.filter(ImageFilter.GaussianBlur(radius=10))
    canvas.alpha_composite(clouds)

    # 5. Top-Down-Jet (rechts oben, klein genug damit der Brand-Block links Platz hat)
    jet_canvas = Image.new("RGBA", (COVER_W, COVER_H), (0, 0, 0, 0))
    # Jet ist 1024x1024-zentriert; verschoben+scaled rechts-oben.
    # SCALE so klein dass die Silhouette unter 230 px hoch bleibt → die Brand-Texte
    # links bleiben frei + der Jet wirkt wie eine elegante Akzent-Ikone.
    SCALE = 0.28
    JET_CX = int(COVER_W * 0.82)
    JET_CY = int(COVER_H * 0.42)
    poly = [(JET_CX + (px - 512) * SCALE, JET_CY + (py - 512) * SCALE) for px, py in JET_POLY]
    # Drop-Shadow (weiches Glow nach unten/links fuer Hoehen-Feeling)
    sh = Image.new("RGBA", (COVER_W, COVER_H), (0, 0, 0, 0))
    sdraw = ImageDraw.Draw(sh)
    sdraw.polygon([(p[0] - 14, p[1] + 18) for p in poly], fill=(0, 0, 0, 130))
    sh = sh.filter(ImageFilter.GaussianBlur(radius=14))
    jet_canvas.alpha_composite(sh)
    # Jet-Body — leicht warmes Weiss (Sunset-licht-reflektiert)
    jd = ImageDraw.Draw(jet_canvas)
    jd.polygon(poly, fill=(252, 248, 240, 235))
    canvas.alpha_composite(jet_canvas)

    # 6. Vignette (cinematic Tiefe an Raendern)
    vignette = Image.new("RGBA", (COVER_W, COVER_H), (0, 0, 0, 0))
    vd = ImageDraw.Draw(vignette)
    for i in range(6):
        vd.rectangle([i * 4, i * 4, COVER_W - i * 4, COVER_H - i * 4],
                     outline=(0, 0, 0, 18 + i * 10), width=4)
    canvas.alpha_composite(vignette)

    # 7. Brand-Block links: Logo + Wordmark + Tagline
    # 7a. Dark-Scrim-Overlay hinter dem Brand-Block damit der Text auf jedem
    #     Hintergrund-Pixel lesbar bleibt — der Sunset-Glow im mittleren Band
    #     hat sonst weisse Schrift weggewischt. Wir bauen einen weichen
    #     Vertical-Gradient von links-volltransparent-schwarz nach
    #     rechts-transparent + Horizontal-Fade damit der Uebergang ins
    #     restliche Bild nahtlos wirkt.
    # Voll-flaechiges dunkles Rechteck — die Fade-Maske MACHT den Uebergang allein.
    # (Erste Version hatte einen senkrechten Cut weil das Rect bei x=560 hart endete
    # WAEHREND die Fade-Maske erst bei x=600 auf 0 ist → 40-px-Diskontinuitaet.)
    scrim = Image.new("RGBA", (COVER_W, COVER_H), (0, 0, 0, 165))
    fade = Image.new("L", (COVER_W, COVER_H), 0)
    fd = ImageDraw.Draw(fade)
    # Horizontaler Soft-Fade-Verlauf: links volldeckend → weicher Tail nach
    # rechts. Laenge des Fade-Tails grosszuegig (340..720), damit kein
    # wahrnehmbarer Edge bleibt — Auge erkennt sub-1%-Alpha-Spruenge gut.
    for x in range(COVER_W):
        if x < 340:
            a = 255
        elif x < 720:
            # Smoothstep (3t² - 2t³) statt linear → noch sanfterer Uebergang
            t = (x - 340) / 380
            sm = t * t * (3 - 2 * t)
            a = int(255 * (1.0 - sm))
        else:
            a = 0
        fd.line([(x, 0), (x, COVER_H)], fill=a)
    scrim_faded = Image.new("RGBA", (COVER_W, COVER_H), (0, 0, 0, 0))
    scrim_faded.paste(scrim, (0, 0), fade)
    canvas.alpha_composite(scrim_faded)

    # 7b. Logo links oben
    if SOURCE_LOGO.exists():
        logo = Image.open(SOURCE_LOGO).convert("RGBA").resize((140, 140), Image.LANCZOS)
        # Sanfter Glow hinter dem Logo damit es vom Hintergrund abgehoben ist
        lg = Image.new("RGBA", (COVER_W, COVER_H), (0, 0, 0, 0))
        ld = ImageDraw.Draw(lg)
        ld.ellipse([50, 50, 220, 220], fill=(0, 0, 0, 110))
        lg = lg.filter(ImageFilter.GaussianBlur(radius=18))
        canvas.alpha_composite(lg)
        canvas.alpha_composite(logo, (60, 60))

    # 7c. Wordmark unter dem Logo
    draw = ImageDraw.Draw(canvas)
    font_wordmark = find_font(82)
    font_tagline = find_font(28, bold=False)
    # Tiefer-Schatten + Hauptschrift in reinweiss — durch den Scrim jetzt
    # auch im Sunset-Band gut lesbar.
    draw.text((64, 220), "AeroACARS", font=font_wordmark, fill=(0, 0, 0, 200))
    draw.text((60, 216), "AeroACARS", font=font_wordmark, fill=(255, 255, 255, 255))

    # Tagline (kurzer Brand-Claim)
    draw.text((63, 327), "Pilot Client · Live-Tracking · Touchdown-Forensik",
              font=font_tagline, fill=(0, 0, 0, 200))
    draw.text((62, 326), "Pilot Client · Live-Tracking · Touchdown-Forensik",
              font=font_tagline, fill=(232, 240, 252, 240))

    # Untere Status-Linie
    font_meta = find_font(22, bold=False)
    draw.text((63, 373), "phpVMS 7 · MSFS 2020/2024 · X-Plane 11/12",
              font=font_meta, fill=(0, 0, 0, 180))
    draw.text((62, 372), "phpVMS 7 · MSFS 2020/2024 · X-Plane 11/12",
              font=font_meta, fill=(196, 214, 234, 230))

    # 8. Save (kein Rounded-Corner — Discord crop't das Cover selbst zu seiner UI)
    out = OUT / "rich_presence_cover.png"
    canvas.save(out, "PNG", optimize=True)
    print(f"OK rich_presence_cover: {out.name}  ({COVER_W}x{COVER_H})")


def main() -> None:
    print("--- AeroACARS Discord-RPC Asset Generator v2 ---\n")
    make_logo()
    # Farb-Konzept: MSFS = Blau-Familie, X-Plane = Gruen-Familie.
    # Jeder Sim hat ein unverkennbares Hintergrund-Motiv (Sterne / Wolken / X)
    # damit man sie auch bei 30x30 px Discord-small_image unterscheiden kann.
    make_sim_badge(
        "sim_msfs2024",
        title="MSFS",  year="2024",
        bg_top=(38, 110, 220), bg_bottom=(10, 30, 90),  # Cobalt → Indigo
        accent=(20, 70, 165),
        bg_motif="stars",
    )
    make_sim_badge(
        "sim_msfs2020",
        title="MSFS",  year="2020",
        bg_top=(60, 110, 180), bg_bottom=(14, 50, 110),  # Steel → Naval
        accent=(30, 70, 140),
        bg_motif="clouds",
    )
    make_sim_badge(
        "sim_xplane12",
        title="X-PLANE", year="12",
        bg_top=(58, 170, 110), bg_bottom=(18, 70, 50),   # Emerald → Forest
        accent=(20, 90, 60),
        bg_motif="x_roundel",
    )
    make_sim_badge(
        "sim_xplane11",
        title="X-PLANE", year="11",
        bg_top=(46, 130, 88), bg_bottom=(14, 55, 38),    # Pine → Deep-Forest
        accent=(18, 70, 50),
        bg_motif="x_roundel",
    )
    make_cover()
    print("\nFertig. 6 PNGs in:", OUT)


if __name__ == "__main__":
    main()
