const { test } = require('node:test');
const assert = require('node:assert/strict');
const { validateVersion, compareVersions, planRelease } = require('./release-plan.cjs');

function api({ release, ref, latest, error } = {}) {
  const response = (value) => async () => {
    if (error) throw error;
    if (value) return { data: value };
    throw Object.assign(new Error('Not found'), { status: 404 });
  };
  return { rest: {
    repos: { getReleaseByTag: response(release), getLatestRelease: response(latest) },
    git: { getRef: response(ref) },
  } };
}

const plan = (options = {}, version = '0.2.0') => planRelease({
  github: api(options), owner: 'Souchy', repo: 'Faust', version, sha: 'test-commit',
});

test('only stable, canonical versions are accepted', () => {
  assert.equal(validateVersion('1.2.3'), '1.2.3');
  for (const version of ['v1.2.3', '01.2.3', '1.2', '1.2.3-beta.1', '1.2.3+build', '', null]) {
    assert.throws(() => validateVersion(version));
  }
});

test('versions compare numerically rather than alphabetically', () => {
  assert.equal(compareVersions('0.10.0', '0.9.9'), 1);
  assert.equal(compareVersions('1.0.0', '1.0.0'), 0);
  assert.equal(compareVersions('1.2.3', '2.0.0'), -1);
});

test('first release and strictly newer versions can be published', async () => {
  assert.deepEqual(await plan(), {
    publish: true, tag: 'v0.2.0', version: '0.2.0', sha: 'test-commit',
    reason: 'Publishing v0.2.0 from test-commit.',
  });
  assert.equal((await plan({ latest: { tag_name: 'v0.1.9' } })).publish, true);
});

test('published releases are skipped, never overwritten', async () => {
  assert.equal((await plan({ release: { draft: false } })).publish, false);
});

test('unfinished drafts and existing tags fail explicitly', async () => {
  await assert.rejects(plan({ release: { draft: true } }), /unfinished draft/);
  await assert.rejects(plan({ ref: { object: { sha: 'other-commit' } } }), /already exists/);
});

test('older or equal versions cannot move the latest release backwards', async () => {
  for (const previous of ['v0.2.0', 'v0.3.0']) {
    await assert.rejects(plan({ latest: { tag_name: previous } }), /must be newer/);
  }
});

test('authentication and network failures are not mistaken for missing releases', async () => {
  for (const status of [401, 403, 429, 500]) {
    await assert.rejects(plan({ error: Object.assign(new Error('API failed'), { status }) }), /API failed/);
  }
});
