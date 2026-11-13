const stableVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

function validateVersion(version) {
  if (typeof version !== 'string' || !stableVersion.test(version)) {
    throw new Error('package.json version must be a stable version such as 0.1.1 (no v prefix or prerelease suffix).');
  }
  return version;
}

function compareVersions(left, right) {
  const a = validateVersion(left).split('.').map(BigInt);
  const b = validateVersion(right).split('.').map(BigInt);
  for (let index = 0; index < 3; index++) {
    if (a[index] !== b[index]) return a[index] > b[index] ? 1 : -1;
  }
  return 0;
}

async function optional(request) {
  try {
    return (await request()).data;
  } catch (error) {
    if (error.status === 404) return null;
    throw error;
  }
}

async function planRelease({ github, owner, repo, version, sha }) {
  validateVersion(version);
  const tag = `v${version}`;
  const release = await optional(() => github.rest.repos.getReleaseByTag({ owner, repo, tag }));
  if (release && !release.draft) {
    return { publish: false, tag, reason: `${tag} is already released; bump package.json to publish another version.` };
  }
  if (release?.draft) {
    throw new Error(`${tag} has an unfinished draft release. Delete that draft and its tag before retrying.`);
  }
  const ref = await optional(() => github.rest.git.getRef({ owner, repo, ref: `tags/${tag}` }));
  if (ref) {
    throw new Error(`${tag} already exists without a published release. Inspect it and remove it only if safe before retrying.`);
  }
  const latest = await optional(() => github.rest.repos.getLatestRelease({ owner, repo }));
  if (latest) {
    const previous = latest.tag_name.replace(/^v/, '');
    if (compareVersions(version, previous) <= 0) {
      throw new Error(`${tag} must be newer than the latest release (${latest.tag_name}).`);
    }
  }
  return { publish: true, tag, version, sha, reason: `Publishing ${tag} from ${sha}.` };
}

module.exports = { validateVersion, compareVersions, planRelease };
