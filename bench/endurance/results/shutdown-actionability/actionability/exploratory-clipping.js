// Local HTTP document: body margin:0; existing <input id=q><button id=go>Go</button>.
// Original exploration: Chrome/CDP click exceeded its 3sec guard;
// Firefox/BiDi before this actionability fix clicked successfully.
// Recorded alongside four Firefox regressions in before-test.log.
window.clicked = 0;
window.ready = true;
window.wrong = 0;
document.querySelector('#go').onclick = e => {
    window.clicked++;
    window.trusted = e.isTrusted;
    if (!window.ready) window.wrong++;
};
const clip = document.createElement('div');
clip.style = 'position:fixed;left:100px;top:100px;width:60px;height:60px;overflow:hidden';
document.body.append(clip);
const button = document.querySelector('#go');
button.style = 'display:block;width:400px;height:50px';
clip.append(button);
// click locator #go and read [window.clicked,window.trusted,window.ready,window.wrong].
// Expected [1,true,true,0]. Separate paired http_clipped_control target owns follow-up.
