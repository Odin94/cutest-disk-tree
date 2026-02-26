const fs = require('fs');
const path = require('path');

const confPath = path.join(__dirname, '..', 'src-tauri', 'tauri.conf.json');
const pubkeyPath = path.join(__dirname, '..', '.tauri-public-key.decoded');
const repoOwner = 'Odin94';
const repoName = 'cutest-disk-tree';

const conf = JSON.parse(fs.readFileSync(confPath, 'utf8'));
const pubkey = fs.readFileSync(pubkeyPath, 'utf8').trim();
conf.plugins = conf.plugins || {};
conf.plugins.updater = conf.plugins.updater || {};
conf.plugins.updater.pubkey = pubkey;
conf.plugins.updater.endpoints = [
  `https://github.com/${repoOwner}/${repoName}/releases/latest/download/latest.json`
];
fs.writeFileSync(confPath, JSON.stringify(conf, null, 2));
