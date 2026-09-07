"use client";

/**
 * The hero: a machine's parts, and their reflection.
 *
 * Eighty-six dots, because that is how many observable entities the machine in
 * the results below actually has. Two rings, one bright and one dim, drifting
 * slowly. It is an illustration and says so in its label; the point is to
 * carry the idea before anybody has read a word.
 *
 * Restraint is the whole brief: no glow, no lines racing between nodes, no
 * particles. Motion slow enough that it reads as breathing rather than
 * activity.
 */

import { useEffect, useRef } from "react";

/** The number of parts the mirror sees on the machine in the results below. */
const ENTITIES = 86;

export function Mirror() {
  const canvas = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    // Fixed seed: the picture is the same on every visit, so it reads as a
    // diagram of one machine rather than as noise.
    let seed = 0x51ed_5eed;
    const random = () => {
      seed ^= seed << 13;
      seed ^= seed >>> 17;
      seed ^= seed << 5;
      return ((seed >>> 0) % 100_000) / 100_000;
    };

    const dots = Array.from({ length: ENTITIES }, (_, index) => ({
      angle: (index / ENTITIES) * Math.PI * 2 + random() * 0.06,
      ring: index < 4 ? 0 : index < 20 ? 1 : 2,
      phase: random() * Math.PI * 2,
      speed: 0.35 + random() * 0.5,
      clarity: 0.28 + random() * 0.7,
    }));

    let frame = 0;
    let running = true;
    const start = performance.now();

    const draw = (now: number) => {
      if (!running) return;
      const context = element.getContext("2d");
      if (!context) return;

      const scale = window.devicePixelRatio || 1;
      const width = element.clientWidth;
      const height = element.clientHeight;
      if (element.width !== width * scale) {
        element.width = width * scale;
        element.height = height * scale;
      }
      context.setTransform(scale, 0, 0, scale, 0, 0);
      context.clearRect(0, 0, width, height);

      const style = getComputedStyle(document.documentElement);
      const ink = style.getPropertyValue("--color-ink").trim() || "#16161a";
      const accent = style.getPropertyValue("--color-accent").trim() || "#1f6f5c";
      const line = style.getPropertyValue("--color-line").trim() || "#e6e6e2";

      const cx = width / 2;
      // The mirror line sits above centre, so the reflection has room.
      const cy = height * 0.42;
      const unit = Math.min(width, height * 1.5) / 2;
      const t = reduced ? 0 : (now - start) / 1000;

      context.strokeStyle = line;
      context.lineWidth = 1;
      for (const ring of [0.34, 0.62, 0.9]) {
        context.beginPath();
        context.arc(cx, cy, unit * ring * 0.62, 0, Math.PI * 2);
        context.stroke();
      }

      // The mirror line.
      context.beginPath();
      context.strokeStyle = line;
      context.moveTo(width * 0.06, cy + unit * 0.62);
      context.lineTo(width * 0.94, cy + unit * 0.62);
      context.stroke();

      for (const dot of dots) {
        const radius = unit * [0.34, 0.62, 0.9][dot.ring]! * 0.62;
        const wobble = Math.sin(t * dot.speed + dot.phase) * 0.045;
        const angle = dot.angle + wobble;
        const x = cx + Math.cos(angle) * radius;
        const y = cy + Math.sin(angle) * radius;
        const size = 1.4 + dot.clarity * 2.4;

        context.beginPath();
        context.fillStyle = rgba(ink, 0.16 + dot.clarity * 0.6);
        context.arc(x, y, size, 0, Math.PI * 2);
        context.fill();

        // Its reflection, below the line, dimmer and inverted.
        const mirrored = cy + unit * 0.62 * 2 - y;
        context.beginPath();
        context.fillStyle = rgba(ink, (0.16 + dot.clarity * 0.6) * 0.17);
        context.arc(x, mirrored, size, 0, Math.PI * 2);
        context.fill();
      }

      context.beginPath();
      context.fillStyle = accent;
      context.arc(cx, cy, 4, 0, Math.PI * 2);
      context.fill();

      frame = requestAnimationFrame(draw);
    };

    frame = requestAnimationFrame(draw);
    return () => {
      running = false;
      cancelAnimationFrame(frame);
    };
  }, []);

  return (
    <div className="relative">
      <canvas
        ref={canvas}
        className="block h-[380px] w-full sm:h-[440px]"
        role="img"
        aria-label="An illustration: the parts of one computer, and their reflection below a line."
      />
    </div>
  );
}

function rgba(colour: string, alpha: number): string {
  if (colour.startsWith("#") && colour.length === 7) {
    const r = parseInt(colour.slice(1, 3), 16);
    const g = parseInt(colour.slice(3, 5), 16);
    const b = parseInt(colour.slice(5, 7), 16);
    return `rgba(${r}, ${g}, ${b}, ${alpha})`;
  }
  return colour;
}
