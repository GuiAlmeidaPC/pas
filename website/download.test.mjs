import { test } from 'node:test';
import assert from 'node:assert/strict';
import { detectOS } from './download.js';

test('detectOS identifies Windows', () => {
  assert.equal(detectOS('Mozilla/5.0 (Windows NT 10.0; Win64; x64)', 'Win32'), 'windows');
});

test('detectOS identifies macOS', () => {
  assert.equal(detectOS('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', 'MacIntel'), 'macos');
});

test('detectOS identifies Linux', () => {
  assert.equal(detectOS('Mozilla/5.0 (X11; Linux x86_64)', 'Linux x86_64'), 'linux');
});

test('detectOS treats Android as linux', () => {
  assert.equal(detectOS('Mozilla/5.0 (Linux; Android 13)', 'Linux armv8l'), 'linux');
});

test('detectOS falls back to linux when unknown', () => {
  assert.equal(detectOS('something weird', ''), 'linux');
});
