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

import { classifyAssets } from './download.js';

const SAMPLE_ASSETS = [
  { name: 'PAS_0.2.0_amd64.AppImage', browser_download_url: 'https://x/app.AppImage' },
  { name: 'PAS_0.2.0_amd64.deb', browser_download_url: 'https://x/app.deb' },
  { name: 'PAS-0.2.0-1.x86_64.rpm', browser_download_url: 'https://x/app.rpm' },
  { name: 'PAS_0.2.0_x64_en-US.msi', browser_download_url: 'https://x/app.msi' },
  { name: 'PAS_0.2.0_x64-setup.exe', browser_download_url: 'https://x/app.exe' },
  { name: 'PAS_0.2.0_universal.dmg', browser_download_url: 'https://x/app.dmg' },
  { name: 'SHA256SUMS.txt', browser_download_url: 'https://x/SHA256SUMS.txt' },
];

test('classifyAssets groups assets by OS via extension', () => {
  const g = classifyAssets(SAMPLE_ASSETS);
  assert.deepEqual(g.windows.map(a => a.name).sort(),
    ['PAS_0.2.0_x64-setup.exe', 'PAS_0.2.0_x64_en-US.msi']);
  assert.deepEqual(g.macos.map(a => a.name), ['PAS_0.2.0_universal.dmg']);
  assert.deepEqual(g.linux.map(a => a.name).sort(),
    ['PAS-0.2.0-1.x86_64.rpm', 'PAS_0.2.0_amd64.AppImage', 'PAS_0.2.0_amd64.deb']);
});

test('classifyAssets ignores non-installer assets like checksums', () => {
  const g = classifyAssets(SAMPLE_ASSETS);
  const all = [...g.windows, ...g.macos, ...g.linux].map(a => a.name);
  assert.equal(all.includes('SHA256SUMS.txt'), false);
});

test('classifyAssets is case-insensitive on extensions', () => {
  const g = classifyAssets([{ name: 'PAS.APPIMAGE', browser_download_url: 'u' }]);
  assert.equal(g.linux.length, 1);
});

test('classifyAssets handles empty/missing input', () => {
  assert.deepEqual(classifyAssets([]), { windows: [], macos: [], linux: [] });
  assert.deepEqual(classifyAssets(undefined), { windows: [], macos: [], linux: [] });
});

import { linuxFormats, findChecksums } from './download.js';

test('linuxFormats returns present formats in order with hints', () => {
  const linux = [
    { name: 'PAS-0.2.0.x86_64.rpm', browser_download_url: 'u-rpm' },
    { name: 'PAS_0.2.0_amd64.AppImage', browser_download_url: 'u-appimage' },
    { name: 'PAS_0.2.0_amd64.deb', browser_download_url: 'u-deb' },
  ];
  const out = linuxFormats(linux);
  assert.deepEqual(out.map(f => f.format), ['AppImage', 'deb', 'rpm']);
  assert.equal(out[0].url, 'u-appimage');
  assert.equal(out[1].url, 'u-deb');
  assert.equal(out[2].url, 'u-rpm');
  assert.ok(out[0].hint.length > 0);
});

test('linuxFormats omits formats not present in the release', () => {
  const linux = [{ name: 'PAS_0.2.0_amd64.AppImage', browser_download_url: 'u' }];
  const out = linuxFormats(linux);
  assert.deepEqual(out.map(f => f.format), ['AppImage']);
});

test('findChecksums returns the SHA256SUMS url or null', () => {
  assert.equal(
    findChecksums([{ name: 'SHA256SUMS.txt', browser_download_url: 'u-sum' }]),
    'u-sum');
  assert.equal(findChecksums([{ name: 'app.deb', browser_download_url: 'u' }]), null);
  assert.equal(findChecksums([]), null);
});

import { primaryAsset } from './download.js';

test('primaryAsset prefers the .exe installer on Windows', () => {
  const win = [
    { name: 'PAS_0.2.0_x64_en-US.msi', browser_download_url: 'u-msi' },
    { name: 'PAS_0.2.0_x64-setup.exe', browser_download_url: 'u-exe' },
  ];
  assert.equal(primaryAsset('windows', win).browser_download_url, 'u-exe');
});

test('primaryAsset falls back to first Windows asset when no .exe exists', () => {
  const win = [{ name: 'PAS_0.2.0_x64_en-US.msi', browser_download_url: 'u-msi' }];
  assert.equal(primaryAsset('windows', win).browser_download_url, 'u-msi');
});

test('primaryAsset returns the first asset for non-Windows OSes', () => {
  const mac = [{ name: 'PAS.dmg', browser_download_url: 'u-dmg' }];
  assert.equal(primaryAsset('macos', mac).browser_download_url, 'u-dmg');
});

test('primaryAsset returns null for an empty/missing list', () => {
  assert.equal(primaryAsset('windows', []), null);
  assert.equal(primaryAsset('linux', undefined), null);
});
