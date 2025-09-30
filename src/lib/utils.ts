import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export const truncate = (text: string) => {
  const str = String(text);
  const chars = Array.from(str);
  if (chars.length <= 24) return str;
  const slice = chars.slice(0, 20).join('');
  const lastSpace = slice.lastIndexOf(' ');
  const base = lastSpace > 0 ? slice.slice(0, lastSpace) : slice;
  return base.replace(/\s+$/u, '') + ' ...';
};

export const cleanChangelog = (body: string | undefined | null): string => {
  if (body === undefined || body === null) {
    return '';
  }
  let result: string = body;
  result = result.replace(/\s*\[#([^\]]+)\]\([^)]*\)/g, '');
  result = result.replace(/\s*\[#([^\]]+)\]/g, '');
  result = result.replace(/ +$/gm, '');
  return result;
};
