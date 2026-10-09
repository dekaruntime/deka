// Execute the built wasm in a real headless browser, independently of website CI.
import { chromium } from 'playwright'
import { createServer } from 'node:http'
import { readFileSync } from 'node:fs'
import { resolve, extname } from 'node:path'
import assert from 'node:assert/strict'
const root = resolve(process.argv[2])
const histories = JSON.parse(readFileSync(resolve(root, 'expectations.json')))
const sources = JSON.parse(readFileSync(resolve(root, 'sources.json')))
function difference(actual, expected, path = 'scene') {
  if (typeof expected === 'number') return typeof actual === 'number' && Math.abs(actual-expected)<.005 ? undefined : `${path}: ${actual} != ${expected}`
  if (Array.isArray(expected)) {
    if (!Array.isArray(actual) || actual.length !== expected.length) return `${path}: array lengths differ`
    for (let i=0; i<expected.length; i++) { const diff=difference(actual[i],expected[i],`${path}[${i}]`); if(diff) return diff }
    return
  }
  if (expected && typeof expected==='object') {
    if (!actual || JSON.stringify(Object.keys(actual).sort())!==JSON.stringify(Object.keys(expected).sort())) return `${path}: object keys differ`
    for (const [key,value] of Object.entries(expected)) { const diff=difference(actual[key],value,`${path}.${key}`); if(diff) return diff }
    return
  }
  return actual===expected ? undefined : `${path}: ${actual} != ${expected}`
}
const server = createServer((req,res)=>{
  const path=new URL(req.url,'http://localhost').pathname
  if (path==='/favicon.ico') {res.writeHead(204);res.end();return}
  if(path==='/') {
    res.setHeader('Content-Type','text/html');res.end(`<!doctype html><div style="position:relative"><canvas tabindex="0" style="width:560px;height:480px"></canvas></div><script type="module">
      const fixture=new URLSearchParams(location.search).get('fixture')||'test-fixture';
      const file=fixture==='input-fixture'?'web_input':fixture==='shared-fixture'?'web_shared':'deka_ui_tour';
      if(fixture==='shared-fixture') document.body.insertAdjacentHTML('beforeend','<div style="position:relative"><canvas tabindex="0" style="width:560px;height:480px"></canvas></div>');
      window.errors=[];window.frames=0;window.dekaFrameTimes=[];
      window.addEventListener('deka:error',event=>window.errors.push(event.detail));
      window.addEventListener('deka:native-frame',event=>{window.scene=event.detail;event.target.dekaScene=event.detail;window.frames++});
      const module=await import('/'+fixture+'/'+file+'.js');await module.default({module_or_path:'/'+fixture+'/'+file+'_bg.wasm'});
      window.closeFirst=module.close_first;
      window.start=lesson=>fixture==='shared-fixture'?module.start(...document.querySelectorAll('canvas')):fixture==='input-fixture'?module.start(document.querySelector('canvas')):module.start(lesson,document.querySelector('canvas'));
      window.stop=module.stop;window.ready=true;
      </script>`);return
  }
  const file=resolve(root,'.'+path)
  if (!file.startsWith(root+'/')) {res.writeHead(404);res.end();return}
  try {res.setHeader('Content-Type',extname(file)==='.wasm'?'application/wasm':'application/javascript');res.end(readFileSync(file))}catch{res.writeHead(404);res.end()}
})
await new Promise(done=>server.listen(0,'127.0.0.1',done))
const url=`http://127.0.0.1:${server.address().port}`
const browser=await chromium.launch({executablePath:process.env.RUST_TOUR_CHROME})
const metrics=[]
try {
  for(const [scale,reduced] of [[1,false],[2,false],[1,true]]) {
    const context=await browser.newContext({deviceScaleFactor:scale,reducedMotion:reduced?'reduce':'no-preference',viewport:{width:1200,height:900}})
    const page=await context.newPage();page.setDefaultTimeout(30000);const errors=[]
    page.on('pageerror',error=>errors.push(error.message));page.on('console',message=>{if(message.type()==='error')errors.push(message.text())})
    await page.goto(url);await page.waitForFunction(()=>window.ready)
    const canvas=page.locator('canvas');const blank=await canvas.screenshot()
    for(const history of histories.filter(h=>h.scale===scale&&h.reduced===reduced)) {
      await page.evaluate(history=>{
        window.stop();window.scene=undefined;window.dekaFrameTimes=[];
        const canvas=document.querySelector('canvas');canvas.style.width=history.width+'px';canvas.style.height=history.height+'px';window.start(history.lesson)
      },history)
      await page.waitForFunction(()=>window.scene || window.errors.length)
      assert.deepEqual(await page.evaluate(()=>window.errors),[],`${history.lesson} render errors`)
      for(const step of history.steps) {
        await canvas.evaluate((canvas,time)=>canvas.dispatchEvent(new CustomEvent('deka:clock',{detail:{time}})),step.time)
        if(step.point) {
          assert.equal(difference(await snapshot(page),step.before),undefined,`${history.lesson} before input`)
          const bounds=await canvas.boundingBox();await page.mouse.click(bounds.x+step.point[0],bounds.y+step.point[1])
        }
        assert.equal(difference(await snapshot(page),step.scene),undefined,`${history.lesson} rendered frame at ${step.time}`)
      }
      assert.notDeepEqual(await canvas.screenshot(),blank,`${history.lesson} must paint visible pixels`)
      // Measure warm frames, including all serialisation across the wasm boundary.
      await page.evaluate(()=>{window.dekaFrameTimes=[];for(let i=0;i<120;i++)document.querySelector('canvas').dispatchEvent(new CustomEvent('deka:clock',{detail:{time:30000+i*16}}))})
      const times=await page.evaluate(()=>window.dekaFrameTimes)
      metrics.push({lesson:history.lesson,scale,reduced,ms:times.map(t=>t.ms),bytes:times.map(t=>t.bytes)})
      await page.evaluate(()=>window.stop());const frames=await page.evaluate(()=>window.frames)
      await canvas.dispatchEvent('pointerup',{button:0,clientX:20,clientY:20})
      await canvas.evaluate(canvas=>canvas.dispatchEvent(new CustomEvent('deka:clock',{detail:{time:99999}})))
      assert.equal(await page.evaluate(()=>window.frames),frames,`${history.lesson} stop removes listeners`)
      assert.deepEqual(await page.evaluate(()=>window.errors),[])
    }
    assert.deepEqual(errors,[])
    console.log(`PASS: all 27 lessons and 8 themed component entries, shared histories, input, pixels and stop; DPR ${scale}, reduced motion ${reduced}`)
    await context.close()
  }
  const page=await browser.newPage();page.setDefaultTimeout(30000)
  // The same production tour bundle/source manifest includes the first
  // component set. Exercise platform DOM keys and canvas pointer ingress.
  const startComponent = async name => {
    await page.goto(url); await page.waitForFunction(()=>window.ready)
    await page.evaluate(name=>window.start(name),name)
    await page.waitForFunction(()=>window.scene || window.errors.length)
    assert.deepEqual(await page.evaluate(()=>window.errors),[])
  }
  await startComponent('component-button-light')
  const action=page.getByRole('button',{name:'Add one',exact:true})
  const actionBounds=await action.boundingBox(); await page.mouse.click(actionBounds.x+8,actionBounds.y+8)
  await action.focus(); await action.press('Enter'); await action.press('Space')
  assert((await page.evaluate(()=>window.scene.nodes)).some(n=>n.text==='Count: 3'))
  assert.equal(await page.getByRole('button',{name:'Unavailable',exact:true}).isDisabled(),true)
  await startComponent('component-input-dark')
  const componentInput=page.getByRole('textbox',{name:'Name',exact:true})
  assert.equal(await componentInput.evaluate(e=>getComputedStyle(e).color),'rgb(245, 239, 226)')
  await componentInput.fill('Ava')
  assert((await page.evaluate(()=>window.scene.nodes)).some(n=>n.text==='Hello Ava'))
  await componentInput.focus(); await page.keyboard.press('Tab'); await page.keyboard.press('Enter')
  assert.equal(await componentInput.inputValue(),'')
  await startComponent('component-list-light')
  const zega=page.getByRole('button',{name:'Zega',exact:true}), zegaId=await zega.getAttribute('id')
  await zega.focus(); await zega.press('Enter')
  assert((await page.evaluate(()=>window.scene.nodes)).some(n=>n.text==='Selected: zega'))
  const reverse=page.getByRole('button',{name:'Reverse order',exact:true})
  await reverse.focus(); await reverse.press('Enter')
  assert.equal(await zega.getAttribute('id'),zegaId)
  assert.deepEqual(await page.getByRole('button').evaluateAll(elements=>elements.map(e=>e.getAttribute('aria-label'))),['cqx','Zega','Deka','Reverse order'])
  await zega.focus(); await page.keyboard.press('Tab')
  assert.equal(await page.getByRole('button',{name:'Deka',exact:true}).evaluate(e=>e===document.activeElement),true)
  await page.keyboard.press('Enter')
  assert((await page.evaluate(()=>window.scene.nodes)).some(n=>n.text==='Selected: deka'))
  await startComponent('component-tabs-dark')
  const overview=page.getByRole('tab',{name:'Overview',exact:true}), activity=page.getByRole('tab',{name:'Activity',exact:true}), settings=page.getByRole('tab',{name:'Settings',exact:true})
  await overview.focus(); await page.keyboard.press('ArrowRight')
  assert.equal(await activity.evaluate(e=>e===document.activeElement),true)
  assert.equal(await activity.getAttribute('aria-selected'),'true')
  assert((await page.evaluate(()=>window.scene.nodes)).some(n=>n.text==='Your latest project activity.'))
  await page.keyboard.press('End'); assert.equal(await settings.evaluate(e=>e===document.activeElement),true)
  await page.keyboard.press('ArrowRight'); assert.equal(await overview.evaluate(e=>e===document.activeElement),true)
  await page.keyboard.press('ArrowLeft'); assert.equal(await settings.evaluate(e=>e===document.activeElement),true)
  await page.keyboard.press('Home'); assert.equal(await overview.evaluate(e=>e===document.activeElement),true)
  assert.equal(await overview.evaluate(e=>document.getElementById(e.getAttribute('aria-controls'))?.getAttribute('role')),'tabpanel')
  assert.deepEqual(await page.evaluate(()=>window.errors),[])
  console.log('PASS: Button/Input/List/Tabs pointer, DOM keyboard, keyed order and accessible tab focus')
  await page.goto(url+'/?fixture=input-fixture');await page.waitForFunction(()=>window.ready);await page.evaluate(()=>window.start())
  const input=page.getByRole('textbox',{name:'Name',exact:true});
  assert.equal(await page.getByRole('textbox',{name:'Notes',exact:true}).count(),1);
  assert.equal(await page.getByRole('button',{name:'Clear',exact:true}).count(),1);
  await input.pressSequentially('Sami')
  assert((await page.evaluate(()=>window.scene.nodes)).some(n=>n.text==='Hello Sami'))
  await input.press('Enter');assert((await page.evaluate(()=>window.scene.nodes)).some(n=>n.text==='Key: Enter'))
  assert.equal(await input.inputValue(),'Confirmed');
  const textarea=page.locator('textarea');assert.equal(await textarea.inputValue(),'Confirmed');
  await textarea.fill('line one\nline two');assert.equal(await input.inputValue(),'line oneline two');
  const editsBefore=await page.evaluate(()=>Number(window.scene.nodes.find(n=>n.text?.startsWith("Edits: ")).text.slice(7)));
  await input.evaluate(input=>{input.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}));input.value='にほん';input.dispatchEvent(new InputEvent('input',{data:'にほん',isComposing:true,bubbles:true}));});
  assert((await page.evaluate(()=>window.scene.nodes)).some(n=>n.text==='Hello line one\nline two'),'preedit must not update signal');
  await input.evaluate(input=>{input.value='日本';input.dispatchEvent(new CompositionEvent('compositionend',{data:'日本',bubbles:true}));input.dispatchEvent(new InputEvent('input',{data:'日本',isComposing:false,bubbles:true}));});
  assert((await page.evaluate(()=>window.scene.nodes)).some(n=>n.text==='Hello 日本'));
  assert.equal(await textarea.inputValue(),'日本');
  assert.equal(await page.evaluate(()=>Number(window.scene.nodes.find(n=>n.text?.startsWith("Edits: ")).text.slice(7))),editsBefore+1,'compositionend and final input form one committed edit');
  await page.context().grantPermissions(['clipboard-read','clipboard-write']);
  await page.evaluate(()=>navigator.clipboard.writeText('clipboard 日本'));
  await input.focus();await input.press('ControlOrMeta+A');await input.press('ControlOrMeta+V');
  assert.equal(await input.inputValue(),'clipboard 日本');
  assert((await page.evaluate(()=>window.scene.nodes)).some(n=>n.text==='Hello clipboard 日本'));
  await input.press('ControlOrMeta+A');await input.press('ControlOrMeta+X');assert.equal(await input.inputValue(),'');
  await input.press('ControlOrMeta+V');assert.equal(await input.inputValue(),'clipboard 日本');
  await input.focus();await page.keyboard.press('Tab');
  assert.equal(await textarea.evaluate(element=>element===document.activeElement),true);
  await page.keyboard.press('Tab');
  assert.equal(await page.getByRole('button',{name:'Clear',exact:true}).evaluate(element=>element===document.activeElement),true);
  await page.keyboard.press('Enter');assert.equal(await input.inputValue(),'');
  await page.getByRole('button',{name:'Focus name',exact:true}).focus()
  await page.keyboard.press('Enter')
  assert.equal(await input.evaluate(element=>element===document.activeElement),true,'retained focus reaches the actual platform text field')
  await input.click({button:'right'});assert((await page.evaluate(()=>window.scene.nodes)).some(node=>node.text==='Key: Context'),'DOM right click reaches the Rust context handler');
  const accessibilitySnapshot=await page.locator('body').ariaSnapshot();
  assert(accessibilitySnapshot.includes('textbox "Name"'));
  assert(accessibilitySnapshot.includes('textbox "Notes"'));
  assert(accessibilitySnapshot.includes('button "Clear"'));
  await page.evaluate(()=>window.stop());assert.equal(await page.getByRole('button',{name:'Clear'}).count(),0);assert.equal(await input.count(),0);assert.equal(await textarea.count(),0)
  console.log('PASS: browser text/key/clipboard input, accessible controls/actions and teardown')
  await page.goto(url+'/?fixture=shared-fixture');await page.waitForFunction(()=>window.ready);await page.evaluate(()=>window.start());
  const canvases=page.locator('canvas');
  const secondBefore=await canvases.nth(1).screenshot();
  await page.getByRole('button',{name:'Add',exact:true}).nth(0).focus();await page.keyboard.press('Enter');
  await page.waitForFunction(()=>[...document.querySelectorAll('canvas')].every(canvas=>canvas.dekaScene.nodes.some(node=>node.text?.endsWith(': 1'))));
  assert.notDeepEqual(await canvases.nth(1).screenshot(),secondBefore,'shared signal repaints the other browser mount');
  await page.evaluate(()=>window.closeFirst());assert.equal(await page.getByRole('button',{name:'Add',exact:true}).count(),1);
  await page.getByRole('button',{name:'Add',exact:true}).focus();await page.keyboard.press('Enter');
  await page.waitForFunction(()=>document.querySelectorAll('canvas')[1].dekaScene.nodes.some(node=>node.text==='Second: 2'));
  await page.evaluate(()=>window.stop());assert.equal(await page.getByRole('button',{name:'Add',exact:true}).count(),0);
  assert.deepEqual(await page.evaluate(()=>window.errors),[]);
  console.log('PASS: shared browser mounts repaint and survive closing the first mount');
  await page.goto(url+'/?fixture=.&inspect');await page.waitForFunction(()=>window.ready);await page.evaluate(()=>window.start('counter'))
  const canvas=page.locator('canvas');const before=await canvas.screenshot()
  await canvas.focus();await page.keyboard.press('Tab');await page.keyboard.press('Enter');assert.notDeepEqual(await canvas.screenshot(),before)
  await canvas.evaluate(canvas=>canvas.dispatchEvent(new CustomEvent('deka:clock',{detail:{time:0}})))
  assert.equal(await page.evaluate(()=>window.frames),0,'production ignores inspect and deterministic clock')
  console.log('PASS: shipped bundle input changes pixels; ?inspect has no effect')
  const largest=sources.reduce((a,b)=>a.source.length>b.source.length?a:b)
  const samples=metrics.filter(m=>m.lesson===largest.id&&m.scale===1&&!m.reduced).flatMap(m=>m.ms).sort((a,b)=>a-b)
  const worst=metrics.reduce((a,b)=>mean(a.ms)>mean(b.ms)?a:b)
  console.log(JSON.stringify({largestSource:largest.id,meanMs:mean(samples),p95Ms:samples[Math.floor(samples.length*.95)],worstMeanLesson:worst.lesson,worstMeanMs:mean(worst.ms),samples:samples.length}))
  assert(mean(samples)<5,'largest lesson must stay below 5ms/frame')
} finally {await browser.close();await new Promise(done=>server.close(done))}
function mean(numbers){return numbers.reduce((a,b)=>a+b,0)/numbers.length}
async function snapshot(page) {return page.evaluate(()=>({...window.scene,images:window.scene.images.map(({rgba,...image})=>({...image,hash:rgba.reduce((hash,byte)=>(Math.imul(hash,33)^byte)>>>0,5381)})).sort((a,b)=>a.id.localeCompare(b.id))}))}
