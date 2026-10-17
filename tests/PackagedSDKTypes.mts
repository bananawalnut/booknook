import {BookNook, type NotebookStorage, type Command} from '@booknook/sdk';
import initWeb, * as core from '@booknook/sdk/wasm/web';
import {recommendations, prepareSyncBatch, sealSyncBatch, openSyncBatch,
  type CatalogCandidate, type SyncBatch, type ApprovedSyncPacket, type Social} from '@booknook/sdk';
const command: Command = {op:'start_session',pass_id:'known-pass',at:1};
declare const storage: NotebookStorage;
await initWeb();
const desk = await BookNook.open(core, storage);
await desk.execute(command);
const drawing: Command = {op:'set_notebook_ink',notebook_id:'notebook',block_id:'block',version_id:'ink-version',
  parents:[],canvas_width:640,canvas_height:320,
  strokes:[{points:[{x:10,y:20,pressure:0.5,time:0}],color:'#315b48',width:2}],native_drawing:null,at:2};
await desk.execute(drawing);
const candidates: CatalogCandidate[] = [{book_id:'catalog:one',title:'Forest',subjects:['nature']}];
recommendations(core,{candidates,preferred_subjects:['nature'],shared_ratings:[],friends:[],excluded:[]});
const batch: SyncBatch = prepareSyncBatch(core,'holder',7n,['device'],new Uint8Array([0,255]));
declare const policy: Social.AuthorityPolicy;
declare const key: Uint8Array;
const packet: ApprovedSyncPacket = sealSyncBatch(core,batch,new Uint8Array([0,255]),key,'request','presentation',policy,110n).packet;
const plaintext: Uint8Array = openSyncBatch(core,packet,key,'device',policy,10000n).plaintext;
void plaintext;
import {verifyCastaliaReceipt, type CastaliaTrustPolicy, type CastaliaDisclosure, type VerifiedCastaliaPresentation} from '@booknook/sdk';
declare const castaliaPolicy: CastaliaTrustPolicy;
declare const disclosure: CastaliaDisclosure;
const verified: VerifiedCastaliaPresentation = verifyCastaliaReceipt(core,'request','presentation','expected',disclosure,castaliaPolicy,110n);
void verified;
import {BookNookServices, type BookNookProvider, type ProviderOperation} from '@booknook/sdk';
declare const provider: BookNookProvider;
const services = await BookNookServices.open(core,storage,[provider]);
const operation: ProviderOperation = {kind:'identity',entity_iri:'urn:reader:one'};
await services.fetch('selected-provider',operation);

const positions = await services.sourcePositions('selected-provider',['source-id']);
await services.attachPosition(positions,{source_id:'source-id',value:'chapter:2',observed_at:1},'known-pass');

import {validatePortableArchive, encodePortableArchive, roundtripPortableArchive, type PortableArchive} from '@booknook/sdk';
const archive: PortableArchive = validatePortableArchive(core, '{"schema":"booknook.portable-export.v1"}');
const portableJSON: string = roundtripPortableArchive(core, encodePortableArchive(core,archive));
void portableJSON;

import {type CapturedReadingContext, type NoteCreation, type NoteMetadata, type CanvasSpace} from '@booknook/sdk';
const captured: CapturedReadingContext = {pass_id:'known-pass',positions:[],position_observations:[]};
const firstInput: NoteCreation = {at:3,capture_context:captured};
const world: CanvasSpace = {kind:'whiteboard',origin_x:-32000,origin_y:-16000};
const modernDrawing: Command = {...drawing,canvas_width:64000,canvas_height:32000,canvas_space:world,capture_context:captured,note_creation:firstInput,at:4};
await desk.execute(modernDrawing);
const face: Command = {op:'set_note_face',notebook_id:'notebook',block_id:'block',face:'text',at:5};
await desk.execute(face);
declare const note: NoteMetadata;
const originalTime: number|null = note.created.at;
void originalTime;
