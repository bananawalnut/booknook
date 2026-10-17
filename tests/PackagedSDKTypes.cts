import {BookNook, type NotebookStorage, type Command, type NotebookInk} from '@booknook/sdk';
import * as core from '@booknook/sdk/wasm/node';
import {socialRecommendations, type CatalogCandidate, type Recommendation, type Social} from '@booknook/sdk';
const command: Command = {op:'create_literary_entity',book_id:'known-book',kind:'place',name:'The library'};
declare const storage: NotebookStorage;
BookNook.open(core, storage).then(desk => desk.execute(command));
declare const ink: NotebookInk;
const transcription: Command = {op:'save_notebook_transcription',notebook_id:'notebook',block_id:'block',
  transcription_id:'reviewed-text',ink_version_id:ink.versions[0].id,text:'Reviewed café',method:'manual',at:2};
BookNook.open(core, storage).then(desk => desk.execute(transcription));
declare const social: Social.SocialState;
declare const policy: Social.AuthorityPolicy;
const candidates: CatalogCandidate[] = [{book_id:'catalog:one',title:'Forest',subjects:[]}];
const suggestions: Recommendation[] = socialRecommendations(core,social,candidates,[],policy,1000n);
void suggestions;
import {validateCastaliaRequest, type CastaliaTrustPolicy, type CastaliaDisclosure, type CastaliaPresentationRequest} from '@booknook/sdk';
declare const castaliaPolicy: CastaliaTrustPolicy;
declare const disclosure: CastaliaDisclosure;
const credential: CastaliaPresentationRequest = validateCastaliaRequest(core,'request','expected',disclosure,castaliaPolicy,110n);
void credential;
import {BookNookServices, type BookNookProvider} from '@booknook/sdk';
declare const provider: BookNookProvider;
BookNookServices.open(core,storage,[provider]).then(service=>service.importCatalog('selected-provider','selected-source'));

import {validatePortableArchive, encodePortableArchive, roundtripPortableArchive, type PortableArchive} from '@booknook/sdk';
const archive: PortableArchive = validatePortableArchive(core, '{"schema":"booknook.portable-export.v1"}');
const portableJSON: string = roundtripPortableArchive(core, encodePortableArchive(core,archive));
void portableJSON;

import {type CapturedReadingContext, type NoteCreation} from '@booknook/sdk';
const captured: CapturedReadingContext = {pass_id:'known-pass',positions:[],position_observations:[]};
const firstInput: NoteCreation = {at:3,capture_context:captured};
const modernText: Command = {op:'edit_block',notebook_id:'notebook',block_id:'block',text:'A thought',base_update:'',at:4,capture_context:captured,note_creation:firstInput};
const addedNote: Command = {op:'add_block',notebook_id:'notebook',at:5,capture_context:captured,face:'whiteboard'};
BookNook.open(core, storage).then(async desk=>{await desk.execute(modernText);await desk.execute(addedNote);});
