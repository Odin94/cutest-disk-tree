const fs = require('fs');
const path = require('path');

const sigPathsFile = process.argv[2];
const outDir = process.argv[3];
const version = process.argv[4];
const pubDate = process.argv[5];
const tag = process.argv[6];
const repoOwner = 'Odin94';
const repoName = 'cutest-disk-tree';

const baseUrl = `https://github.com/${repoOwner}/${repoName}/releases/download/${tag}`;

const platformFromFilename = (name) => {
  if (/\.(msi|exe)$/i.test(name)) return 'windows-x86_64';
  if (/\.dmg$/i.test(name)) return /aarch64|arm64/i.test(name) ? 'darwin-aarch64' : 'darwin-x86_64';
  if (/\.(app\.tar\.gz|tar\.gz)$/i.test(name)) return /aarch64|arm64/i.test(name) ? 'darwin-aarch64' : 'darwin-x86_64';
  if (/\.AppImage$/i.test(name)) return 'linux-x86_64';
  if (/\.deb$/i.test(name)) return 'linux-x86_64';
  return null;
};

const sigPaths = fs.readFileSync(sigPathsFile, 'utf8').trim().split('\n').filter(Boolean);
const platforms = {};
for (const sigPath of sigPaths) {
  const sigContent = fs.readFileSync(sigPath, 'utf8').trim();
  const assetName = path.basename(sigPath, '.sig');
  const platform = platformFromFilename(assetName);
  if (!platform) continue;
  platforms[platform] = { signature: sigContent, url: baseUrl + '/' + encodeURIComponent(assetName) };
}

const out = { version, notes: '', pub_date: pubDate, platforms };
fs.writeFileSync(path.join(outDir, 'latest.json'), JSON.stringify(out, null, 2));
