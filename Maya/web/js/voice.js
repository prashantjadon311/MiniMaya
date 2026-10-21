import {request} from './api.js';
import {confirmApproval} from './dom.js';

let stream;
let recorder;
let chunks=[];
let captureSettings;
let captureTimer;
let blurHandler;

async function sha256(bytes){
  const digest=await crypto.subtle.digest('SHA-256',bytes);
  return [...new Uint8Array(digest)].map(value=>value.toString(16).padStart(2,'0')).join('');
}

async function approvedVoiceOperation(payload){
  const receipt=await request('POST','/actions',{kind:'voice_upload',...payload});
  if(!receipt.approval)throw new Error('Maya did not create a voice approval request');
  const decision=await confirmApproval(receipt.approval);
  const approval=await request('POST',`/approvals/${receipt.approval.id}/decision`,{
    revision:receipt.approval.revision,
    request_digest:receipt.approval.request_digest,
    decision,
    owner_ack:true
  });
  if(approval.decision!=='approved')throw new Error('Voice transfer was denied');
  return approval;
}

function releaseCapture(){
  clearTimeout(captureTimer);
  captureTimer=undefined;
  if(blurHandler)window.removeEventListener('blur',blurHandler);
  blurHandler=undefined;
  stream?.getTracks().forEach(track=>track.stop());
  stream=undefined;
  recorder=undefined;
  captureSettings=undefined;
}

export async function startCapture(settings){
  if(recorder)throw new Error('Voice capture is already active');
  if(!settings?.stt_model_id)throw new Error('Configure a speech-to-text model first');
  stream=await navigator.mediaDevices.getUserMedia({audio:settings.input_device?{deviceId:{exact:settings.input_device}}:true});
  chunks=[];
  captureSettings=settings;
  recorder=new MediaRecorder(stream);
  recorder.ondataavailable=event=>{if(event.data.size)chunks.push(event.data)};
  recorder.start();
  blurHandler=()=>stop();
  window.addEventListener('blur',blurHandler,{once:true});
  captureTimer=setTimeout(()=>stop(),60000);
}

export async function stopCapture(){
  if(!recorder)throw new Error('Voice capture is not active');
  const activeRecorder=recorder;
  const settings=captureSettings;
  const stopped=new Promise((resolve,reject)=>{
    activeRecorder.addEventListener('stop',resolve,{once:true});
    activeRecorder.addEventListener('error',event=>reject(event.error||new Error('Voice capture failed')),{once:true});
  });
  activeRecorder.stop();
  await stopped;
  const audio=new Blob(chunks,{type:activeRecorder.mimeType||'audio/webm'});
  releaseCapture();
  const bytes=await audio.arrayBuffer();
  const payload_digest=await sha256(bytes);
  const approval=await approvedVoiceOperation({payload_digest,size_bytes:audio.size,model_id:settings.stt_model_id,mode:'stt'});
  const data=new FormData();
  data.append('audio',audio,'voice.webm');
  data.append('meta',JSON.stringify({language:settings.language||'english',model_id:settings.stt_model_id,approval_id:approval.id,payload_digest}));
  return request('POST','/voice/transcriptions',data);
}

export async function speak(text,settings){
  if(settings.muted)return;
  if((settings.tts==='browser'||!settings.tts_model_id)&&'speechSynthesis'in window){
    const utterance=new SpeechSynthesisUtterance(text);
    utterance.rate=settings.rate||1;
    if(settings.voice){const selected=speechSynthesis.getVoices().find(item=>item.name===settings.voice);if(selected)utterance.voice=selected;}
    speechSynthesis.speak(utterance);
    return;
  }
  const encoded=new TextEncoder().encode(text);
  const payload_digest=await sha256(encoded);
  const approval=await approvedVoiceOperation({payload_digest,size_bytes:encoded.byteLength,model_id:settings.tts_model_id,mode:'tts'});
  const output=await request('POST','/voice/speech',{text,model_id:settings.tts_model_id,voice:settings.voice||'default',rate:settings.rate||1,language:settings.language||'english',approval_id:approval.id});
  if(output.audio_artifact_id){
    const blob=await request('GET',`/artifacts/${output.audio_artifact_id}`,undefined,{binary:true});
    const url=URL.createObjectURL(blob);
    const audio=new Audio(url);
    audio.addEventListener('ended',()=>URL.revokeObjectURL(url),{once:true});
    audio.addEventListener('error',()=>URL.revokeObjectURL(url),{once:true});
    await audio.play();
  }
}

export function stop(){
  if(recorder?.state==='recording')recorder.stop();
  releaseCapture();
  window.speechSynthesis?.cancel();
}

export async function devices(){return navigator.mediaDevices.enumerateDevices();}
