#include "custody_policy.h"
#include <stdlib.h>
#include <string.h>

#define LIMIT (128U*1024U)
struct parser { const char* source;size_t size,position,length;char* output;const char* root; };
static int append(struct parser* p,const char* value,size_t size) {
  if(size>LIMIT-p->length)return 0;
  memcpy(p->output+p->length,value,size);p->length+=size;return 1;
}
static int text(struct parser* p,const char* value) { return append(p,value,strlen(value)); }
static int whitespace(char c) { return c==' '||c=='\t'||c=='\n'||c=='\r'; }
static void skip(struct parser* p) {
  while(p->position<p->size) {
    char c=p->source[p->position];
    if(whitespace(c)){++p->position;continue;}
    if(c==';'){while(p->position<p->size&&p->source[p->position]!='\n')++p->position;continue;}
    break;
  }
}
static int token(struct parser* p,size_t* begin,size_t* size) {
  skip(p);*begin=p->position;
  while(p->position<p->size) {
    char c=p->source[p->position];
    if(whitespace(c)||c=='('||c==')'||c==';'||c=='"')break;
    ++p->position;
  }
  *size=p->position-*begin;return *size>0;
}
static int equal(struct parser* p,size_t begin,size_t size,const char* value) {
  return strlen(value)==size&&!memcmp(p->source+begin,value,size);
}
static int root_valid(const char* root) {
  if(!root||root[0]!='/'||root[1]==0||strlen(root)>4096||root[strlen(root)-1]=='/')return 0;
  const char* component=root+1;
  for(const char* c=component;;++c) {
    if(*c=='/'||!*c) {
      size_t size=(size_t)(c-component);
      if(!size||(size==1&&component[0]=='.')||(size==2&&!memcmp(component,"..",2)))return 0;
      if(!*c)break;component=c+1;
    } else if((unsigned char)*c<32||*c=='"'||*c=='\\')return 0;
  }
  return 1;
}
static int exclusion(struct parser* p) {
  return text(p,"(require-not (subpath \"")&&text(p,p->root)&&text(p,"\"))");
}
static int security_exclusion(struct parser* p) {
  return text(p,"(require-not (require-any "
    "(global-name-regex #\"^com[.]apple[.](SecurityServer|securityd([.].*)?|trustd([.].*)?)$\") "
    "(xpc-service-name-regex #\"^com[.]apple[.](SecurityServer|securityd([.].*)?|trustd([.].*)?)$\")))");
}
static int expression(struct parser* p,unsigned depth) {
  if(depth>64||p->position>=p->size||p->source[p->position]!='(')return 0;
  size_t begin=p->position++,output_begin=p->length,op_begin=0,op_size=0;
  if(!token(p,&op_begin,&op_size)&&(p->position>=p->size||p->source[p->position]!='('))return 0;
  int files=0,mach=0,remove=0;
  if(equal(p,op_begin,op_size,"allow")) {
    unsigned file_count=0,mach_count=0,other_count=0,extension_count=0;
    while(1) {
      size_t saved=p->position;skip(p);
      if(p->position==p->size)return 0;
      if(p->source[p->position]=='('||p->source[p->position]==')'){p->position=saved;break;}
      size_t operation=0,size=0;if(!token(p,&operation,&size))return 0;
      if(equal(p,operation,size,"file-issue-extension"))++extension_count;
      else if(equal(p,operation,size,"mach-lookup"))++mach_count;
      else if((size>=5&&!memcmp(p->source+operation,"file-",5))||equal(p,operation,size,"file*"))++file_count;
      else ++other_count;
    }
    if((file_count&&(other_count||mach_count))||(mach_count&&other_count)||(extension_count&&(file_count||other_count||mach_count)))return 0;
    files=file_count>0;mach=mach_count>0;remove=extension_count>0;
  }
  if(!append(p,p->source+begin,p->position-begin))return 0;
  unsigned filters=0;
  while(p->position<p->size) {
    size_t segment=p->position;skip(p);
    if(!append(p,p->source+segment,p->position-segment)||p->position==p->size)return 0;
    char c=p->source[p->position];
    if(c==')') {
      if((files||mach)&&!filters&&(!text(p," ")||!(files?exclusion(p):security_exclusion(p))))return 0;
      ++p->position;if(!text(p,")"))return 0;
      if(remove){p->length=output_begin;return text(p,"#f");}return 1;
    }
    if(c=='(') {
      size_t saved=p->position++,child_begin=0,child_size=0;
      if(!token(p,&child_begin,&child_size)&&(p->position>=p->size||p->source[p->position]!='('))return 0;
      int modifier=equal(p,child_begin,child_size,"with");p->position=saved;
      if((files||mach)&&!modifier) {
        ++filters;
        if(!text(p,"(require-all ")||!(files?exclusion(p):security_exclusion(p))||!text(p," ")||!expression(p,depth+1)||!text(p,")"))return 0;
      } else if(!expression(p,depth+1))return 0;
      continue;
    }
    if(files||mach)return 0; /* Predicates/modifiers must be complete expressions. */
    segment=p->position;
    if(c=='"') {
      ++p->position;int ended=0;
      while(p->position<p->size) {
        char value=p->source[p->position++];
        if(value=='\\'){if(p->position==p->size)return 0;++p->position;}
        else if(value=='"'){ended=1;break;}
      }
      if(!ended)return 0;
    } else {
      while(p->position<p->size) {
        c=p->source[p->position];
        if(whitespace(c)||c=='('||c==')'||c==';')break;
        if(c=='"')break;++p->position;
      }
      if(p->position==segment)return 0;
    }
    if(!append(p,p->source+segment,p->position-segment))return 0;
  }
  return 0;
}
int colossus_owned_custody_policy(const char* source,size_t size,const char* root,char** output) {
  if(!output)return 0;*output=NULL;
  if(!source||!size||size>LIMIT||memchr(source,0,size)||!root_valid(root))return 0;
  struct parser p={source,size,0,0,malloc(LIMIT+1),root};if(!p.output)return 0;
  while(p.position<size) {
    size_t before=p.position;skip(&p);
    if(!append(&p,source+before,p.position-before))goto fail;
    if(p.position==size)break;
    /* The preceding process transformer replaces removed top-level grants
     * with Scheme false. Accept only that exact closed sentinel. */
    if(size-p.position>=2&&!memcmp(source+p.position,"#f",2)&&
       (p.position+2==size||whitespace(source[p.position+2])||source[p.position+2]==';')) {
      if(!append(&p,source+p.position,2))goto fail;p.position+=2;continue;
    }
    if(!expression(&p,0))goto fail;
  }
  if(!text(&p,"\n; Immutable custody namespace exclusion applies to every positive file grant.\n(deny file-read* file-write* (subpath \"")||
     !text(&p,root)||!text(&p,"\"))\n(deny file-issue-extension)\n"))goto fail;
  p.output[p.length]=0;*output=p.output;return 1;
fail:free(p.output);return 0;
}
