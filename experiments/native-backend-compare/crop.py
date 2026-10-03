import struct,zlib,sys
def read(p):
    d=open(p,'rb').read(); i=8; w=h=0; idat=b''
    while i<len(d):
        n=struct.unpack('>I',d[i:i+4])[0]; t=d[i+4:i+8]; c=d[i+8:i+8+n]; i+=12+n
        if t==b'IHDR': w,h=struct.unpack('>II',c[:8])
        if t==b'IDAT': idat+=c
    raw=zlib.decompress(idat); bpp=4; stride=w*4; out=bytearray(); prev=bytearray(stride); pos=0
    for y in range(h):
        f=raw[pos]; pos+=1; line=bytearray(raw[pos:pos+stride]); pos+=stride
        for x in range(stride):
            a=line[x-bpp] if x>=bpp else 0; b=prev[x]; c=prev[x-bpp] if x>=bpp else 0
            if f==1: line[x]=(line[x]+a)&255
            elif f==2: line[x]=(line[x]+b)&255
            elif f==3: line[x]=(line[x]+(a+b)//2)&255
            elif f==4:
                p=a+b-c; pa,pb,pc=abs(p-a),abs(p-b),abs(p-c)
                line[x]=(line[x]+(a if pa<=pb and pa<=pc else b if pb<=pc else c))&255
        out+=line; prev=line
    return w,h,out
def write(p,w,h,px):
    raw=b''.join(b'\0'+bytes(px[y*w*4:(y+1)*w*4]) for y in range(h))
    def chunk(t,c): return struct.pack('>I',len(c))+t+c+struct.pack('>I',zlib.crc32(t+c)&0xffffffff)
    open(p,'wb').write(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw,6))+chunk(b'IEND',b''))
def crops(names,x,y,cw,ch,z,out):
    imgs=[read(n) for n in names]; W=(cw*z+8)*len(imgs); H=ch*z; px=bytearray(b'\xff'*(W*H*4))
    for k,(w,h,d) in enumerate(imgs):
        for oy in range(H):
            for ox in range(cw*z):
                s=((y+oy//z)*w+x+ox//z)*4; t=(oy*W+k*(cw*z+8)+ox)*4; px[t:t+4]=d[s:s+4]
    write(out,W,H,px)
if __name__=='__main__':
    a=sys.argv; crops(a[1].split(','),int(a[2]),int(a[3]),int(a[4]),int(a[5]),int(a[6]),a[7])
