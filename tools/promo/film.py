"""Films the race on the website into a 1920x1080 promo video.

Usage: python3 tools/promo/film.py http://localhost:8765/index.html out.mp4
Needs Playwright with a Chromium/Chrome and ffmpeg. The page must be served with ?film support
(site/index.html exposes window.raceFrame(t)).
"""
import asyncio, os, subprocess, sys, tempfile

from playwright.async_api import async_playwright

URL, OUT = sys.argv[1], sys.argv[2]
FPS = 30
CHROME = os.environ.get("CHROME", "/usr/bin/google-chrome")

TITLE_JS = """(p) => {
  const h = document.querySelector('.headline');
  const [l1, l2] = [h.querySelector('h1').childNodes[0], h.querySelector('.hot')];
  const e = (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3);
  const h1 = h.querySelector('h1');
  h1.style.transform = `scale(${1.18 - 0.18 * e(p / 2.4)})`;
  h1.style.opacity = e(p / 0.5);
  const hot = h.querySelector('.hot');
  hot.style.display = 'inline-block';
  hot.style.transform = `translateY(${(1 - e((p - 0.7) / 0.5)) * 60}px)`;
  hot.style.opacity = e((p - 0.7) / 0.4);
  h.querySelector('.lead').style.opacity = e((p - 1.3) / 0.5);
  h.querySelector('.cta').style.opacity = e((p - 1.6) / 0.5);
}"""

END_JS = """(p) => {
  let d = document.getElementById('endcard');
  if (!d) {
    d = document.createElement('div'); d.id = 'endcard';
    d.style.cssText = 'position:fixed;inset:0;z-index:99;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:18px;text-align:center;color:#fff;' +
      'background:radial-gradient(900px 420px at 50% 120%, rgba(255,106,19,.45), transparent 70%), #1d1633;font-family:Bricolage,system-ui';
    d.innerHTML = '<div style="font:800 120px/1 Bricolage;font-stretch:78%;letter-spacing:-.04em">ferro<span style="color:#ff6a13">report</span></div>' +
      '<div style="font:650 40px/1.2 Bricolage;color:#ffe08a">Same .jrxml. Same PDF. No JVM.</div>' +
      '<div style="font:500 26px/1.4 system-ui;color:#d9d3f0;margin-top:18px">Free and open source<br>cristianencarnacion.github.io/ferroreport</div>';
    document.body.appendChild(d);
  }
  const e = (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3);
  d.style.opacity = e(p / 0.45);
  d.firstChild.style.transform = `scale(${1.25 - 0.25 * e(p / 0.9)})`;
}"""


async def scene(page, path, seconds, step):
    tmp = path + ".part.mp4"
    enc = subprocess.Popen(
        ["ffmpeg", "-y", "-loglevel", "error", "-f", "image2pipe", "-framerate", str(FPS), "-c:v", "mjpeg", "-i", "-",
         "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", "-r", str(FPS), tmp],
        stdin=subprocess.PIPE)
    for i in range(round(seconds * FPS)):
        await step(i / FPS)
        enc.stdin.write(await page.screenshot(type="jpeg", quality=92))
    enc.stdin.close()
    if enc.wait() != 0:
        raise SystemExit("ffmpeg failed on " + path)
    os.replace(tmp, path)


async def main():
    work = tempfile.mkdtemp()
    async with async_playwright() as p:
        browser = await p.chromium.launch(executable_path=CHROME)
        page = await browser.new_page(viewport={"width": 1280, "height": 720}, device_scale_factor=1.5)
        await page.goto(URL + "?film")
        await page.wait_for_timeout(1200)
        await page.evaluate("raceFrame(0)")

        await scene(page, f"{work}/1.mp4", 2.6, lambda t: page.evaluate(TITLE_JS, t))

        top = await page.evaluate("document.getElementById('race').getBoundingClientRect().top + scrollY")
        await page.evaluate(f"scrollTo(0, {top - 46})")
        await scene(page, f"{work}/2.mp4", 6.4, lambda t: page.evaluate("t => raceFrame(t)", t + 0.2))

        await scene(page, f"{work}/3.mp4", 3.2, lambda t: page.evaluate(END_JS, t))
        await browser.close()

    with open(f"{work}/list.txt", "w") as f:
        f.write("".join(f"file '{work}/{i}.mp4'\n" for i in (1, 2, 3)))
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", f"{work}/list.txt",
                    "-c", "copy", "-movflags", "+faststart", OUT], check=True)
    print(OUT)


asyncio.run(main())
