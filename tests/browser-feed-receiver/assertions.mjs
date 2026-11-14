// SPDX-License-Identifier: MIT OR Apache-2.0
// Runs only inside a disposable automated-browser context supplied by the host.
import assert from 'node:assert/strict';
const source = (id, title) => ({id, title, url: `https://feeds.example.org/${id}.xml`, format: 'rss', enabled: true, collectionIds: [], disclosure: 'selected'});
const ids = ['d3e42c42-32f7-4c43-a0bb-555555555551', 'd3e42c42-32f7-4c43-a0bb-555555555552', 'd3e42c42-32f7-4c43-a0bb-555555555553'];
const section = subscriptions => ({schema: 'castalia.feeds-section.v1', version: 1, subscriptions, collections: [], preferences: {order: 'newest-first'}});
const config = (revision, subscriptions) => ({schema: 'booknook.feed-configuration.v1', revision, feeds: section(subscriptions)});
export async function runFeedReceiverAssertions(context, receiverURL) {
  const page = await context.newPage(); await page.goto(receiverURL); await page.evaluate(() => window.feedReceiverTest.ready);
  const first = source(ids[0], '<img src="https://tracker.example/leak" onerror="window.executed=true"> **literal**');
  const second = source(ids[1], 'Existing private settings'); second.disclosure = 'private';
  await page.evaluate(raw => window.feedReceiverTest.seedConfiguration(raw), JSON.stringify(config(5, [first, second])));
  const before = await page.evaluate(() => window.feedReceiverTest.read());
  const incoming = ' \n' + JSON.stringify(section([{...first, title: 'Incoming interpretation'}, source(ids[2], 'New source')])) + '\n';
  const preview = await page.evaluate(raw => window.feedReceiverTest.receive(raw), incoming);
  assert.equal(preview.candidate, null); assert.equal(preview.conflicts.length, 1);
  assert.equal((await page.evaluate(() => window.feedReceiverTest.state)).raw, incoming);
  assert.deepEqual(await page.evaluate(() => window.feedReceiverTest.read()), before);
  assert.equal(await page.locator('#conflicts img').count(), 0); assert.equal(await page.evaluate(() => window.executed), undefined);
  await assert.rejects(page.evaluate(() => window.feedReceiverTest.resolve([])));
  assert.deepEqual(await page.evaluate(() => window.feedReceiverTest.read()), before);
  await page.locator('#conflicts select').selectOption('current'); await page.locator('#resolve-conflicts').click();
  await page.locator('#confirm-import:not([disabled])').waitFor();
  assert.equal((await page.evaluate(() => window.feedReceiverTest.state)).preview.candidate.feeds.subscriptions.length, 3);
  await page.locator('#cancel-import').click(); assert.deepEqual(await page.evaluate(() => window.feedReceiverTest.read()), before);
  await page.evaluate(raw => window.feedReceiverTest.receive(raw), incoming);
  await page.locator('#conflicts select').selectOption('incoming'); await page.locator('#resolve-conflicts').click();
  await page.locator('#confirm-import').click();
  await page.waitForFunction(() => window.feedReceiverTest.state.result?.changed === true && !window.feedReceiverTest.state.pending);
  const accepted = await page.evaluate(() => window.feedReceiverTest.read());
  assert.equal(accepted.config.revision, 6); assert.equal(accepted.config.feeds.subscriptions.length, 3);
  assert.equal(accepted.config.feeds.subscriptions.find(s => s.id === ids[0])?.title, 'Incoming interpretation');
  assert.equal(accepted.config.feeds.subscriptions.find(s => s.id === ids[1])?.disclosure, 'private');
  assert.deepEqual((await page.evaluate(() => window.feedReceiverTest.state)).activations, []);
  // Empty subsets and exact repeats neither delete nor rewrite an accepted row.
  for (const raw of [JSON.stringify(section([])), JSON.stringify(accepted.config.feeds)]) {
    await page.evaluate(value => window.feedReceiverTest.receive(value), raw);
    const result = await page.evaluate(() => window.feedReceiverTest.confirm());
    assert.deepEqual(result, {accepted: true, changed: false});
    assert.deepEqual(await page.evaluate(() => window.feedReceiverTest.read()), accepted);
  }
  // The receiving host preserves its independently stricter URL/parser policy.
  for (const raw of [
    JSON.stringify({...section([]), schema: {'castalia.feeds-section.v1': null}}),
    '{"schema":"castalia.feeds-section.v1","schema":"castalia.feeds-section.v1"}',
    JSON.stringify({schema: 'booknook.selected-feeds.v1', origin: 'https://reader.example.org', feeds: section([])}),
    JSON.stringify(section([{...first, url: 'https://feeds.example.org/feed?private=interest'}])),
  ]) {
    await assert.rejects(page.evaluate(value => window.feedReceiverTest.receive(value), raw));
    assert.deepEqual(await page.evaluate(() => window.feedReceiverTest.read()), accepted);
  }
  // Another actual tab commits after the first confirmation check but before
  // its transaction. The final IDB comparison must refuse the stale candidate.
  const other = await context.newPage(); await other.goto(receiverURL); await other.evaluate(() => window.feedReceiverTest.ready);
  const extra = source('d3e42c42-32f7-4c43-a0bb-555555555554', 'A pending addition');
  await page.evaluate(raw => window.feedReceiverTest.receive(raw), JSON.stringify(section([extra])));
  await page.evaluate(() => window.feedReceiverTest.armCommitBarrier());
  await page.evaluate(() => {window.pendingFixtureCommit = window.feedReceiverTest.confirm().then(value => ({ok: true, value}), error => ({ok: false, error: error.message}));});
  await page.evaluate(() => window.feedReceiverTest.waitCommitBarrier());
  const competing = structuredClone(accepted.config); competing.revision++;
  competing.feeds.subscriptions[0].title = 'Concurrent tab wins';
  await other.evaluate(raw => window.feedReceiverTest.seedConfiguration(raw), JSON.stringify(competing));
  await page.evaluate(() => window.feedReceiverTest.releaseCommitBarrier());
  const refused = await page.evaluate(() => window.pendingFixtureCommit); assert.equal(refused.ok, false); assert.match(refused.error, /Storage changed/);
  const finalRow = await other.evaluate(() => window.feedReceiverTest.read()); assert.deepEqual(finalRow.config, competing);
  await page.evaluate(() => window.feedReceiverTest.cancel());
  // Cancel after preparation but before the transaction is a local no-write.
  await page.evaluate(raw => window.feedReceiverTest.receive(raw), JSON.stringify(section([extra])));
  await page.evaluate(() => window.feedReceiverTest.armCommitBarrier());
  await page.evaluate(() => {window.pendingFixtureCommit = window.feedReceiverTest.confirm().then(value => ({ok: true, value}), error => ({ok: false, error: error.message}));});
  await page.evaluate(() => window.feedReceiverTest.waitCommitBarrier());
  await page.evaluate(() => {window.feedReceiverTest.cancel(); window.feedReceiverTest.releaseCommitBarrier();});
  assert.equal((await page.evaluate(() => window.pendingFixtureCommit)).ok, false);
  assert.deepEqual(await page.evaluate(() => window.feedReceiverTest.read()), finalRow);
  await page.reload(); await page.evaluate(() => window.feedReceiverTest.ready);
  assert.deepEqual(await page.evaluate(() => window.feedReceiverTest.read()), finalRow);
  assert.deepEqual((await page.evaluate(() => window.feedReceiverTest.state)).activations, []);
  await other.close(); await page.close();
  return {passed: ['opaque original-byte import', 'all conflicts explicit', 'cancel without save', 'additive selected interpretation', 'private records retained', 'empty/no-op row retained', 'malformed/wrapper/strict-URL refusal', 'actual competing-tab CAS', 'pre-CAS cancellation', 'restart inactive persistence'], fixtureOnly: true, runtimeOrProductionAcceptance: false};
}
