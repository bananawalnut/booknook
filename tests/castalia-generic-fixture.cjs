// SPDX-License-Identifier: MIT OR Apache-2.0
// Public deterministic test identities only. No runtime signing or custody implementation.
const {createPrivateKey, createPublicKey, createHash, sign} = require('node:crypto');
function canonical(value) {
  if (Array.isArray(value)) return '[' + value.map(canonical).join(',') + ']';
  if (value && typeof value === 'object') return '{' + Object.keys(value).sort().map(k => JSON.stringify(k) + ':' + canonical(value[k])).join(',') + '}';
  return JSON.stringify(value);
}
function digest(value) { return createHash('sha256').update(value).digest('hex'); }
function key(seed) {
  const privateKey = createPrivateKey({key:Buffer.concat([Buffer.from('302e020100300506032b657004220420','hex'),Buffer.alloc(32,seed)]),format:'der',type:'pkcs8'});
  return {privateKey, publicKey:createPublicKey(privateKey).export({format:'der',type:'spki'}).subarray(-32).toString('hex')};
}
function signed(privateKey, domain, value) { return sign(null,Buffer.from(domain + canonical(value)),privateKey).toString('hex'); }
function genericFixture() {
  const issuer=key(41), holder=key(42), caller={kind:'terminal',id:'synthetic-castalia-second-app'};
  const expectedRequest=canonical({schema:'synthetic.castalia-read-request.v1',document:'test:one',operation:'read',nonce:'test-once'});
  const disclosure={title:'Read the selected test document',statements:['This synthetic request belongs to a second application.']};
  const policy={pins:[{issuer:'test-issuer',key_id:'test-key',public_key:issuer.publicKey,audience:'synthetic-second-app',callers:[caller]}],
    expected_caller:caller,audience:'synthetic-second-app',holder_public_key:holder.publicKey,policy_digest:digest('second app policy')};
  const claims={schema:'castalia.request-credential.v1',issuer:'test-issuer',key_id:'test-key',holder_public_key:holder.publicKey,
    audience:policy.audience,caller,request_digest_sha256:digest(expectedRequest),disclosure_digest_sha256:digest(canonical(disclosure)),
    policy_digest_sha256:policy.policy_digest,nonce:'11'.repeat(16),issued_at:100,expires_at:220};
  const credential={claims,signature:signed(issuer.privateKey,'castalia.request-credential.v1/signature\0',claims)};
  const request={schema:'castalia.credential-presentation-request.v2',request_bytes_base64:Buffer.from(expectedRequest).toString('base64'),disclosure,credential};
  const presentation={schema:'castalia.credential-presentation.v2',holder_public_key:holder.publicKey,issuer:claims.issuer,key_id:claims.key_id,
    audience:policy.audience,caller,request_digest_sha256:claims.request_digest_sha256,credential_digest_sha256:digest(canonical(credential)),
    disclosure_digest_sha256:claims.disclosure_digest_sha256,nonce:'22'.repeat(16),issued_at:101,expires_at:161};
  presentation.signature=signed(holder.privateKey,'castalia.credential-presentation.v2/signature\0',presentation);
  return {request:canonical(request),presentation:canonical(presentation),expectedRequest,disclosure,policy,now:110n};
}
module.exports={genericFixture};
