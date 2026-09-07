/**
 * The computer, looking at itself.
 *
 * # What is actually drawn
 *
 * One dot per entity in the mirror. Its opacity is how much of that entity
 * CoreScout can currently observe, and its size is how much is happening
 * there. Nothing is decorative: a dim dot means a real gap in what this
 * machine exposes, and the Computer screen will name it.
 *
 * # Why a canvas and not a graph library
 *
 * Eighty-six dots redrawn a few times a second is not a visualisation
 * problem. A layout library would bring more code than the whole application
 * and animate on every frame whether or not anything changed, which is the one
 * cost this product cannot pay.
 *
 * # Restraint
 *
 * No glow, no particles, no lines racing between nodes. The motion is a slow
 * ease of each dot towards its new size, and a ring that appears when the
 * machine enters a state it recognises. Anything more and this becomes a
 * screensaver that happens to be attached to a database.
 */

import { useEffect, useRef } from "react";
import type { Node } from "./api";

interface Placed {
  x: number;
  y: number;
  radius: number;
  target: number;
  alpha: number;
  targetAlpha: number;
}

/** Rings, outermost first, by the class of entity. */
const ORDER = ["machine", "package", "numa", "core", "cpu", "cache", "irq"];

function ring(node: Node): number {
  const index = ORDER.indexOf(node.class);
  return index === -1 ? ORDER.length : index;
}

export function MirrorView({
  nodes,
  stateId,
  seen,
  plain,
  unfamiliar,
}: {
  nodes: Node[];
  stateId?: number;
  seen: number;
  plain: string;
  unfamiliar: boolean;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const placed = useRef<Map<string, Placed>>(new Map());
  const pulse = useRef(0);
  const lastState = useRef<number | undefined>(undefined);

  // A state change is the one event worth animating: it is the machine
  // recognising itself, which is the whole idea.
  useEffect(() => {
    if (stateId !== lastState.current) {
      pulse.current = 1;
      lastState.current = stateId;
    }
  }, [stateId]);

  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    let frame = 0;
    let running = true;

    const draw = () => {
      if (!running) return;
      const context = element.getContext("2d");
      if (!context) return;

      const scale = window.devicePixelRatio || 1;
      const width = element.clientWidth;
      const height = element.clientHeight;
      if (element.width !== width * scale || element.height !== height * scale) {
        element.width = width * scale;
        element.height = height * scale;
      }
      context.setTransform(scale, 0, 0, scale, 0, 0);
      context.clearRect(0, 0, width, height);

      const style = getComputedStyle(document.documentElement);
      const ink = style.getPropertyValue("--ink").trim() || "#16161a";
      const accent = style.getPropertyValue("--accent").trim() || "#1f6f5c";
      const line = style.getPropertyValue("--line").trim() || "#e6e6e2";

      const cx = width / 2;
      const cy = height / 2;
      const radius = Math.min(width, height) / 2 - 28;

      // Rings, drawn faintly, so the structure reads even before any dot
      // moves. Their number comes from the machine, not from a constant.
      const rings = new Set(nodes.map(ring));
      context.strokeStyle = line;
      context.lineWidth = 1;
      for (const level of rings) {
        const r = radius * ((level + 1) / (ORDER.length + 1));
        context.beginPath();
        context.arc(cx, cy, r, 0, Math.PI * 2);
        context.stroke();
      }

      // Place each dot once and keep it there, so the picture is stable
      // between frames and the eye can follow one entity.
      const byRing = new Map<number, Node[]>();
      for (const node of nodes) {
        const level = ring(node);
        const list = byRing.get(level) ?? [];
        list.push(node);
        byRing.set(level, list);
      }

      for (const [level, members] of byRing) {
        const r = radius * ((level + 1) / (ORDER.length + 1));
        members.forEach((node, index) => {
          const angle = (index / Math.max(members.length, 1)) * Math.PI * 2 - Math.PI / 2;
          let dot = placed.current.get(node.key);
          if (!dot) {
            dot = { x: 0, y: 0, radius: 0, target: 0, alpha: 0, targetAlpha: 0 };
            placed.current.set(node.key, dot);
          }
          dot.x = cx + Math.cos(angle) * r;
          dot.y = cy + Math.sin(angle) * r;
          dot.target = 1.8 + node.activity * 5;
          // Clarity is how much of this entity is readable. A dim dot is a
          // real gap, not a styling choice.
          dot.targetAlpha = 0.18 + node.clarity * 0.72;
          dot.radius += (dot.target - dot.radius) * 0.08;
          dot.alpha += (dot.targetAlpha - dot.alpha) * 0.08;

          context.beginPath();
          context.fillStyle = withAlpha(ink, dot.alpha);
          context.arc(dot.x, dot.y, dot.radius, 0, Math.PI * 2);
          context.fill();
        });
      }

      // The centre: the machine as one thing, and the pulse when it
      // recognises the state it is in.
      if (pulse.current > 0.01) {
        context.beginPath();
        context.strokeStyle = withAlpha(accent, pulse.current * 0.5);
        context.lineWidth = 1.5;
        context.arc(cx, cy, 10 + (1 - pulse.current) * radius * 0.9, 0, Math.PI * 2);
        context.stroke();
        pulse.current *= 0.955;
      }

      context.beginPath();
      context.fillStyle = stateId === undefined ? withAlpha(ink, 0.25) : accent;
      context.arc(cx, cy, 4.5, 0, Math.PI * 2);
      context.fill();

      frame = requestAnimationFrame(draw);
    };

    frame = requestAnimationFrame(draw);
    return () => {
      running = false;
      cancelAnimationFrame(frame);
    };
  }, [nodes, stateId]);

  return (
    <div className="mirror">
      <canvas ref={canvas} aria-hidden="true" />
      <div className="caption">
        <strong>{plain}</strong>
        {stateId === undefined ? (
          <span>Every dot is a part of this machine. Brighter means CoreScout can see more of it.</span>
        ) : unfamiliar ? (
          <span>This is new. CoreScout has nothing to compare it to yet.</span>
        ) : (
          <span>
            State {stateId}, seen {seen} times before.
          </span>
        )}
      </div>
    </div>
  );
}

/** A recent history of how busy the machine has been. */
export function Sparkline({ values }: { values: number[] }) {
  const canvas = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const context = element.getContext("2d");
    if (!context) return;

    const scale = window.devicePixelRatio || 1;
    const width = element.clientWidth;
    const height = element.clientHeight;
    element.width = width * scale;
    element.height = height * scale;
    context.setTransform(scale, 0, 0, scale, 0, 0);
    context.clearRect(0, 0, width, height);
    if (values.length < 2) return;

    const style = getComputedStyle(document.documentElement);
    const accent = style.getPropertyValue("--accent").trim() || "#1f6f5c";
    // Scaled to its own range rather than to zero: the interesting thing about
    // this line is its shape, and a fixed zero flattens it into nothing.
    const top = Math.max(...values);
    const bottom = Math.min(...values);
    const span = top - bottom || 1;

    context.beginPath();
    context.strokeStyle = accent;
    context.lineWidth = 1.5;
    context.lineJoin = "round";
    values.forEach((value, index) => {
      const x = (index / (values.length - 1)) * width;
      const y = height - ((value - bottom) / span) * (height - 4) - 2;
      if (index === 0) context.moveTo(x, y);
      else context.lineTo(x, y);
    });
    context.stroke();
  }, [values]);

  return <canvas className="sparkline" ref={canvas} aria-hidden="true" />;
}

/** Apply an alpha to a colour that may be hex or already a function. */
function withAlpha(colour: string, alpha: number): string {
  const clamped = Math.max(0, Math.min(1, alpha));
  if (colour.startsWith("#") && (colour.length === 7 || colour.length === 4)) {
    const full =
      colour.length === 4
        ? `#${colour[1]}${colour[1]}${colour[2]}${colour[2]}${colour[3]}${colour[3]}`
        : colour;
    const r = parseInt(full.slice(1, 3), 16);
    const g = parseInt(full.slice(3, 5), 16);
    const b = parseInt(full.slice(5, 7), 16);
    return `rgba(${r}, ${g}, ${b}, ${clamped})`;
  }
  return colour;
}
