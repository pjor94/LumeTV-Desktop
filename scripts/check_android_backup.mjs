// Independent Android BackupCipher v1 vectors, using Node's OpenSSL crypto implementation.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve, dirname } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createCipheriv, createDecipheriv, pbkdf2Sync } from 'node:crypto';
import assert from 'node:assert/strict';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const folder = resolve(root, 'tests/fixtures/android-backup');
const password = 'password Android';
const header = Buffer.concat([Buffer.from('LUMEBK01'), Buffer.from(Array.from({length:24},(_,i)=>i))]);
const derive = (head) => pbkdf2Sync(Buffer.from(password,'utf8'),head.subarray(8,24),600000,32,'sha256');
const archive = execFileSync('python3', ['-c', `import zipfile,io,pathlib,sys
p=pathlib.Path(sys.argv[1]);output=io.BytesIO()
with zipfile.ZipFile(output,'w',compression=zipfile.ZIP_DEFLATED) as z:
 for name,file in [('lists/user_'+'a'*32+'.jsonl','list.jsonl'),('manifest.json','manifest.json')]:
  info=zipfile.ZipInfo(name,date_time=(1980,1,1,0,0,0));info.compress_type=zipfile.ZIP_DEFLATED;z.writestr(info,(p/file).read_bytes())
sys.stdout.buffer.write(output.getvalue())`, folder]);
const frames = [header];let index=0;
for(let offset=0;;offset+=256*1024) {
 const payload=archive.subarray(offset,Math.min(archive.length,offset+256*1024));const meta=Buffer.alloc(8);meta.writeUInt32BE(index,0);meta.writeUInt32BE(payload.length,4);
 const iv=Buffer.concat([header.subarray(24),meta.subarray(0,4)]);const cipher=createCipheriv('aes-256-gcm',derive(header),iv);cipher.setAAD(Buffer.concat([header,meta]));
 frames.push(meta.subarray(4),cipher.update(payload),cipher.final(),cipher.getAuthTag());index++;
 if(payload.length===0)break;
}
const vector=Buffer.concat(frames);const target=resolve(folder,'android-v1.lumebackup.base64');
if(process.argv.includes('--generate'))writeFileSync(target,vector.toString('base64')+'\n');
else assert.deepEqual(Buffer.from(readFileSync(target,'utf8').trim(),'base64'),vector,'The Android wire fixture drifted. Review the format before regenerating.');
const encrypted=Buffer.from(readFileSync(target,'utf8').trim(),'base64');const head=encrypted.subarray(0,32);let offset=32;index=0;const plain=[];
while(true){const size=encrypted.readUInt32BE(offset);offset+=4;const meta=Buffer.alloc(8);meta.writeUInt32BE(index,0);meta.writeUInt32BE(size,4);const decipher=createDecipheriv('aes-256-gcm',derive(head),Buffer.concat([head.subarray(24),meta.subarray(0,4)]));decipher.setAAD(Buffer.concat([head,meta]));decipher.setAuthTag(encrypted.subarray(offset+size,offset+size+16));plain.push(decipher.update(encrypted.subarray(offset,offset+size)),decipher.final());offset+=size+16;index++;if(size===0)break;}
assert.equal(offset,encrypted.length);assert.deepEqual(Buffer.concat(plain),archive);
console.log('Android BackupCipher v1: independent encryption/decryption vector verified.');
