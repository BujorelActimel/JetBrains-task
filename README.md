# Solution Breakdown

## Problem statement

Write a client app that makes a get request to a buggy http server and check if the response is  
correct by checking if the sha-256 encoding of the response is the same as the one printed by the  
server.

## So what's the challange?

The problem is that if the response has more than 64 * 1024 bytes (wich is guaranteed, as the data   
is between 512 * 1024 and 1024^2 bytes) the data that gets sent as a response is truncated to a random  
size between 64 * 1024 bytes and the full length. Here is a simple sketch showing how would that look:  

![simple get request example](./images/simple-req.svg)

So, if we want to receive a predictable amount of data, we have to get it in chunks of 64 * 1024 bytes
at a time, at most. This can be achieved by adding the **Range** header to the server request. With  
it we specify what part of the data to get. Here is how you would get the first chunk of the data:

![range get request example](./images/range-req.svg)

To get the whole data from the server we have to make multiple chunk requests and append the responses.  
This could take up to 16 seconds (in the worst the data has the max length of 1024*1024 bytes  
and every request takes 1 second, so 16 chunks of 64 * 1024 bytes with a second per chunk - 16s). 
That if we make the requests one after the other, but if we make them concurently in batches of 8  
we could take the total response time down to two seconds. 

![range get request batch example](./images/batch-range-req.svg)

The problem with that would be that appending responses wouldn't be as simple as before, because  
of the random response times (and even if the response times would be the same, the correct ordering  
still wouldn't be guaranteed because of how concurency works). 

![concurent responses](./images/concurent-responses.svg)

So to solve that problem, we should associate some kind of id to each request so that we can  
re-arrange the responses in the correct order after receiveing all of them.

![sorted concurent responses](./images/sorted-concurent-responses.svg)
