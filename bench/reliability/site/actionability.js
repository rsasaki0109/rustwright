// Shared deterministic click scenarios; success requires the real click event.
window.prepareActionability = (kind, delay) => {
  const host = document.querySelector('#host');
  host.replaceChildren();
  window.clicked = false;
  window.clickedBeforeReady = false;
  window.wrongClicks = 0;
  window.actionReady = false;
  const button = document.createElement('button');
  button.id = 'target';
  button.textContent = 'Submit';
  button.style.cssText = 'position:fixed;left:80px;top:80px;width:100px;height:40px';
  const accept = () => {
    window.clicked = true;
    window.clickedBeforeReady = !window.actionReady;
  };
  button.onclick = accept;
  host.append(button);
  const uncover = () => {
    const cover = document.createElement('div');
    cover.id = 'cover';
    cover.style.cssText = 'position:fixed;inset:0;z-index:100;background:rgba(0,0,0,.1)';
    cover.onclick = () => { window.wrongClicks++; };
    host.append(cover);
    setTimeout(() => { cover.remove(); window.actionReady = true; }, delay);
  };
  if (kind === 'disabled') {
    button.disabled = true;
    setTimeout(() => { button.disabled = false; window.actionReady = true; }, delay);
  } else if (kind === 'fieldset') {
    const fieldset = document.createElement('fieldset');
    fieldset.disabled = true;
    host.append(fieldset);
    fieldset.append(button);
    setTimeout(() => { fieldset.disabled = false; window.actionReady = true; }, delay);
  } else if (kind === 'aria') {
    host.setAttribute('aria-disabled', 'true');
    setTimeout(() => { host.removeAttribute('aria-disabled'); window.actionReady = true; }, delay);
  } else if (kind === 'covered') {
    uncover();
  } else if (kind === 'moving') {
    button.animate([{transform:'translateX(0)'}, {transform:'translateX(250px)'}],
      {duration:delay, fill:'forwards'}).finished.then(() => { window.actionReady = true; });
  } else if (kind === 'hover_cover') {
    button.addEventListener('mouseenter', uncover, {once:true});
  } else if (kind === 'hover_replace') {
    button.addEventListener('mouseenter', () => {
      const replacement = button.cloneNode(true);
      replacement.onclick = accept;
      button.replaceWith(replacement);
      window.actionReady = true;
    }, {once:true});
  } else {
    window.actionReady = true;
  }
};

window.prepareGeometry = (kind) => {
  const host = document.querySelector('#host');
  host.replaceChildren();
  window.clicked = false;
  window.clickedBeforeReady = false;
  window.wrongClicks = 0;
  window.actionReady = true;
  const target = document.createElement('button');
  target.id = 'target';
  target.textContent = 'Geometry';
  target.style.cssText = 'position:fixed;left:80px;top:80px;width:180px;height:60px;border:0;padding:0';
  target.onclick = () => { window.clicked = true; };
  host.append(target);
  if (kind === 'partial_cover' || kind === 'rotated_partial_cover') {
    if (kind === 'rotated_partial_cover') target.style.transform = 'rotate(45deg)';
    const cover = document.createElement('div');
    cover.id = 'cover';
    cover.style.cssText = 'position:fixed;left:140px;top:80px;width:60px;height:60px;z-index:100';
    cover.onclick = () => { window.wrongClicks++; };
    host.append(cover);
  } else if (kind === 'clipped') {
    host.style.cssText = 'position:fixed;left:50px;top:100px;width:150px;height:60px;overflow:hidden';
    target.style.cssText = 'width:1000px;height:60px;border:0;padding:0';
  } else if (kind === 'rotated_clipped') {
    target.style.cssText = 'position:fixed;left:-150px;top:100px;width:200px;height:30px;border:0;padding:0;transform:rotate(45deg)';
  } else if (kind === 'perspective') {
    target.style.cssText += ';transform:perspective(120px) rotateY(45deg) rotateZ(20deg);transform-origin:0 0';
  } else if (kind === 'clip_path') {
    target.style.clipPath = 'polygon(0 0, 10% 0, 10% 100%, 0 100%)';
  }
};
