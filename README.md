# Load Balancer Rust
Simple high-performance TCP-level load balancer made in Rust

## Why?
Because sometimes you just need a very simple tool with minimal configuration and that just works.

Also because I just wanted to try using Rust.

## Issues
Not yet fully optimized for Windows. Some weird behavior causing slower response times than on Linux.

## Performance testing
Some load testing was done using the [k6](https://k6.io/) tool to get an idea of relative performance. All testing was done on a system running Ubuntu 20.04.

The performed test was defined as:
```js
import http from 'k6/http';
import { sleep } from 'k6';

export let options = {
  stages: [
    { duration: '20s', target: 1000 },  // slowly ramp-up traffic from 1 to 1000 users over 20 seconds
    { duration: '1m', target: 1000 },   // remain at 1000 users for 1 minute
    { duration: '20s', target: 0 },     // slowly ramp-down to 0 users
  ]
};

const BASE_URL = 'http://localhost:7777';

export default () => {
  let res = http.get(`${BASE_URL}/test_endpoint`);
  sleep(1);
};
```

### Reference
The test was first ran directly against a local web server to get a reference point:
![](https://cryshana.me/f/T2bwGCVdYM04.png)

Average response time was around **0.25ms**.

### nginx 1.18.0
I then set up a reverse proxy on nginx like so:
```nginx
server {
  listen 6666;
  listen [::]:6666;
  
  location / {
    proxy_pass http://localhost:5000
  }
}
```
And ran the test against nginx and got the following results:
![](https://cryshana.me/f/uVmlKwSzzRJm.png)

Average response time was around **0.55ms**.

### Load Balancer Rust
And then I tried using my own tool - using just one host to test reverse proxying performance:

![](https://cryshana.me/f/CAXD08i5DyaH.png)

Average response time was around **0.36ms**. (About **52% faster** than nginx)
